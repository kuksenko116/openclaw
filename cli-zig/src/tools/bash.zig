const std = @import("std");
const Allocator = std.mem.Allocator;
const types = @import("../types.zig");

const max_output_bytes: usize = 8000;

/// Shared state between the main thread and timeout watcher.
/// Uses atomics to avoid data races.
const TimeoutState = struct {
    timed_out: std.atomic.Value(bool) = std.atomic.Value(bool).init(false),
    /// Set to true by main thread when child has been reaped.
    /// The watcher checks this before killing to avoid killing a reused PID.
    child_done: std.atomic.Value(bool) = std.atomic.Value(bool).init(false),
};

/// Execute a bash command, capturing stdout and stderr.
///
/// Spawns `/bin/bash -c <command>` with pipe-based output capture.
/// Implements timeout via a separate thread that kills the child process.
/// Output is truncated to 8000 bytes.
pub fn execute(allocator: Allocator, command: []const u8, timeout_ms: u64) !types.ToolResult {
    var child = std.process.Child.init(
        &.{ "/bin/bash", "-c", command },
        allocator,
    );
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;

    try child.spawn();

    // Shared timeout state on the heap so it outlives both threads safely
    var state = TimeoutState{};

    // Spawn timeout watcher thread
    const timer_thread = if (timeout_ms > 0) std.Thread.spawn(.{}, timeoutWatcher, .{
        child.id,
        timeout_ms,
        &state,
    }) catch null else null;

    // Read stdout and stderr (with embedded size limit)
    const stdout = collectOutput(allocator, child.stdout.?);
    const stderr = collectOutput(allocator, child.stderr.?);
    defer {
        if (stdout) |s| allocator.free(s);
        if (stderr) |s| allocator.free(s);
    }

    // Wait for child to fully exit (reaps zombie)
    const term = child.wait();

    // Signal to watcher that the child is reaped — prevents killing a reused PID
    state.child_done.store(true, .release);

    // Join timer thread so we don't leak it
    if (timer_thread) |t| t.join();

    const timed_out = state.timed_out.load(.acquire);

    const exit_code: ?u32 = if (term) |t| switch (t) {
        .Exited => |code| @intCast(code),
        else => null,
    } else |_| null;

    return formatResult(allocator, stdout orelse "", stderr orelse "", exit_code, timed_out);
}

fn collectOutput(allocator: Allocator, file: std.fs.File) ?[]const u8 {
    var buf = std.ArrayList(u8).init(allocator);
    errdefer buf.deinit();

    var read_buf: [4096]u8 = undefined;
    while (true) {
        const n = file.read(&read_buf) catch break;
        if (n == 0) break;
        const space = max_output_bytes -| buf.items.len;
        if (space == 0) break;
        buf.appendSlice(read_buf[0..@min(n, space)]) catch break;
    }

    return buf.toOwnedSlice() catch null;
}

fn timeoutWatcher(pid: std.process.Child.Id, timeout_ms: u64, state: *TimeoutState) void {
    // Sleep in small increments so we can exit early if child finishes
    const sleep_interval = 50 * std.time.ns_per_ms; // 50ms
    var remaining_ns: u64 = timeout_ms * std.time.ns_per_ms;

    while (remaining_ns > 0) {
        // If child already exited, no need to wait further
        if (state.child_done.load(.acquire)) return;

        const sleep_time = @min(remaining_ns, sleep_interval);
        std.time.sleep(sleep_time);
        remaining_ns -|= sleep_time;
    }

    // Check one more time — child may have finished during our last sleep
    if (state.child_done.load(.acquire)) return;

    // Timeout expired — mark and kill
    state.timed_out.store(true, .release);
    std.posix.kill(pid, std.posix.SIG.KILL) catch {};
}

fn formatResult(
    allocator: Allocator,
    stdout: []const u8,
    stderr: []const u8,
    exit_code: ?u32,
    timed_out: bool,
) !types.ToolResult {
    var result = std.ArrayList(u8).init(allocator);
    errdefer result.deinit();
    const w = result.writer();

    if (stdout.len > 0) {
        try w.writeAll(stdout);
    }

    if (stderr.len > 0) {
        if (stdout.len > 0) try w.writeAll("\n");
        try w.writeAll("[stderr]\n");
        try w.writeAll(stderr);
    }

    if (timed_out) {
        try w.writeAll("\n[timed out]");
    }

    if (exit_code) |code| {
        if (code != 0) {
            try w.print("\n[exit code: {d}]", .{code});
        }
    }

    const content = try result.toOwnedSlice();
    const is_error = timed_out or (exit_code != null and exit_code.? != 0);

    return .{
        .content = content,
        .is_error = is_error,
    };
}

// -- Tests --

test "execute runs a simple command" {
    const allocator = std.testing.allocator;
    const result = try execute(allocator, "echo hello", 5000);
    defer allocator.free(result.content);
    try std.testing.expect(!result.is_error);
    try std.testing.expect(std.mem.startsWith(u8, result.content, "hello"));
}

test "execute captures exit code on failure" {
    const allocator = std.testing.allocator;
    const result = try execute(allocator, "exit 42", 5000);
    defer allocator.free(result.content);
    try std.testing.expect(result.is_error);
    try std.testing.expect(std.mem.indexOf(u8, result.content, "exit code: 42") != null);
}

test "execute captures stderr" {
    const allocator = std.testing.allocator;
    const result = try execute(allocator, "echo err >&2", 5000);
    defer allocator.free(result.content);
    try std.testing.expect(std.mem.indexOf(u8, result.content, "[stderr]") != null);
    try std.testing.expect(std.mem.indexOf(u8, result.content, "err") != null);
}
