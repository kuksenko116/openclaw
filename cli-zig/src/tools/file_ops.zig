const std = @import("std");
const Allocator = std.mem.Allocator;
const types = @import("../types.zig");

const default_line_limit: usize = 2000;
const max_line_len: usize = 2000;
const max_file_size: usize = 10 * 1024 * 1024; // 10 MB

/// Truncate a UTF-8 string to at most `max_len` bytes on a valid character boundary.
fn truncateUtf8(s: []const u8, max_len: usize) []const u8 {
    if (s.len <= max_len) return s;
    // Walk backwards from max_len to find a valid UTF-8 char boundary.
    // A byte is a start byte if it doesn't match 10xxxxxx (continuation byte).
    var end = max_len;
    while (end > 0 and (s[end] & 0xC0) == 0x80) : (end -= 1) {}
    return s[0..end];
}

// ---- readFile ----

/// Read a file and return its contents with line numbers (cat -n style).
/// `offset` is 1-based line number to start from. `limit` is max lines.
pub fn readFile(
    allocator: Allocator,
    path: []const u8,
    offset: ?usize,
    limit: ?usize,
) !types.ToolResult {
    const file = std.fs.openFileAbsolute(path, .{}) catch |err| {
        return errorResult(allocator, "Error reading file: {s}", .{@errorName(err)});
    };
    defer file.close();

    const stat = file.stat() catch |err| {
        return errorResult(allocator, "Error stating file: {s}", .{@errorName(err)});
    };
    if (stat.kind == .directory) {
        return .{ .content = "Cannot read a directory. Use glob or bash 'ls' instead.", .is_error = true };
    }

    const start_line = offset orelse 1;
    const max_lines = limit orelse default_line_limit;

    var output = std.ArrayList(u8).init(allocator);
    errdefer output.deinit();
    const writer = output.writer();

    var buffered = std.io.bufferedReader(file.reader());
    var reader = buffered.reader();

    var line_number: usize = 0;
    var lines_written: usize = 0;

    while (true) {
        const line = reader.readUntilDelimiterOrEofAlloc(allocator, '\n', max_file_size) catch |err| {
            if (err == error.StreamTooLong) break;
            return errorResult(allocator, "Error reading line: {s}", .{@errorName(err)});
        };
        if (line == null) break;
        defer allocator.free(line.?);

        line_number += 1;
        if (line_number < start_line) continue;
        if (lines_written >= max_lines) break;

        const display = truncateUtf8(line.?, max_line_len);
        try writer.print("{d:>6}\t{s}\n", .{ line_number, display });
        lines_written += 1;
    }

    return .{ .content = try output.toOwnedSlice(), .is_error = false };
}

// ---- writeFile ----

/// Write content to a file. Creates parent directories if needed.
pub fn writeFile(
    allocator: Allocator,
    path: []const u8,
    content: []const u8,
) !types.ToolResult {
    // Ensure parent directory exists
    if (std.fs.path.dirname(path)) |dir| {
        std.fs.makeDirAbsolute(dir) catch |err| {
            switch (err) {
                error.PathAlreadyExists => {},
                else => {
                    return errorResult(allocator, "Error creating directory: {s}", .{@errorName(err)});
                },
            }
        };
    }

    const file = std.fs.createFileAbsolute(path, .{}) catch |err| {
        return errorResult(allocator, "Error creating file: {s}", .{@errorName(err)});
    };
    defer file.close();

    file.writeAll(content) catch |err| {
        return errorResult(allocator, "Error writing file: {s}", .{@errorName(err)});
    };

    const msg = try std.fmt.allocPrint(allocator, "Wrote {d} bytes to {s}", .{ content.len, path });
    return .{ .content = msg, .is_error = false };
}

// ---- editFile ----

/// Find and replace an exact string in a file.
/// Fails if old_string is not found or appears more than once.
pub fn editFile(
    allocator: Allocator,
    path: []const u8,
    old_string: []const u8,
    new_string: []const u8,
) !types.ToolResult {
    const content = readEntireFile(allocator, path) catch |err| {
        return errorResult(allocator, "Error reading file: {s}", .{@errorName(err)});
    };
    defer allocator.free(content);

    // Find first occurrence
    const first = std.mem.indexOf(u8, content, old_string) orelse {
        return .{ .content = "old_string not found in file", .is_error = true };
    };

    // Check uniqueness
    const second = std.mem.indexOfPos(u8, content, first + old_string.len, old_string);
    if (second != null) {
        return .{
            .content = "old_string is not unique in file. Provide more context or use replace_all.",
            .is_error = true,
        };
    }

    // Build replacement
    var result = std.ArrayList(u8).init(allocator);
    errdefer result.deinit();
    try result.appendSlice(content[0..first]);
    try result.appendSlice(new_string);
    try result.appendSlice(content[first + old_string.len ..]);

    // Write back
    const out_file = std.fs.createFileAbsolute(path, .{}) catch |err| {
        return errorResult(allocator, "Error writing file: {s}", .{@errorName(err)});
    };
    defer out_file.close();
    out_file.writeAll(result.items) catch |err| {
        return errorResult(allocator, "Error writing file: {s}", .{@errorName(err)});
    };

    return .{ .content = "Edit applied successfully", .is_error = false };
}

// ---- globFiles ----

/// Simple glob using filesystem walk with pattern matching.
/// Returns matching file paths, one per line.
pub fn globFiles(
    allocator: Allocator,
    pattern: []const u8,
    base_path: ?[]const u8,
) !types.ToolResult {
    const base = base_path orelse ".";

    var dir = std.fs.openDirAbsolute(base, .{ .iterate = true }) catch |err| {
        return errorResult(allocator, "Error opening directory: {s}", .{@errorName(err)});
    };
    defer dir.close();

    var output = std.ArrayList(u8).init(allocator);
    errdefer output.deinit();
    const writer = output.writer();

    var walker = try dir.walk(allocator);
    defer walker.deinit();

    var count: usize = 0;
    const max_results: usize = 1000;

    while (try walker.next()) |entry| {
        if (count >= max_results) {
            try writer.print("\n[truncated: showing first {d} results]", .{max_results});
            break;
        }
        if (matchGlob(pattern, entry.path)) {
            try writer.print("{s}/{s}\n", .{ base, entry.path });
            count += 1;
        }
    }

    if (count == 0) {
        return .{ .content = "No files matched the pattern.", .is_error = false };
    }

    return .{ .content = try output.toOwnedSlice(), .is_error = false };
}

// ---- grepFiles ----

/// Search for a pattern in files by shelling out to grep.
/// Returns matching lines with file paths and line numbers.
pub fn grepFiles(
    allocator: Allocator,
    pattern: []const u8,
    path: ?[]const u8,
    include: ?[]const u8,
) !types.ToolResult {
    var args = std.ArrayList([]const u8).init(allocator);
    defer args.deinit();

    try args.append("grep");
    try args.append("-rn");
    try args.append("--color=never");

    var inc_flag: ?[]const u8 = null;
    defer if (inc_flag) |f| allocator.free(f);

    if (include) |inc| {
        inc_flag = try std.fmt.allocPrint(allocator, "--include={s}", .{inc});
        try args.append(inc_flag.?);
    }

    try args.append(pattern);
    try args.append(path orelse ".");

    var child = std.process.Child.init(args.items, allocator);
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Pipe;

    child.spawn() catch |err| {
        return errorResult(allocator, "Error spawning grep: {s}", .{@errorName(err)});
    };

    const stdout = collectPipeOutput(allocator, child.stdout.?) catch |err| {
        _ = child.wait() catch {};
        return errorResult(allocator, "Error reading grep output: {s}", .{@errorName(err)});
    };
    defer allocator.free(stdout);

    _ = child.wait() catch {};

    if (stdout.len == 0) {
        return .{ .content = "No matches found.", .is_error = false };
    }

    // Truncate long output
    if (stdout.len > 8000) {
        const truncated = try std.fmt.allocPrint(
            allocator,
            "{s}\n\n[truncated: {d} bytes omitted]",
            .{ stdout[0..8000], stdout.len - 8000 },
        );
        return .{ .content = truncated, .is_error = false };
    }

    return .{ .content = try allocator.dupe(u8, stdout), .is_error = false };
}

// ---- Helpers ----

fn readEntireFile(allocator: Allocator, path: []const u8) ![]const u8 {
    const file = try std.fs.openFileAbsolute(path, .{});
    defer file.close();
    return file.readToEndAlloc(allocator, max_file_size);
}

fn collectPipeOutput(allocator: Allocator, file: std.fs.File) ![]const u8 {
    var buf = std.ArrayList(u8).init(allocator);
    errdefer buf.deinit();
    var read_buf: [4096]u8 = undefined;
    while (true) {
        const n = file.read(&read_buf) catch break;
        if (n == 0) break;
        try buf.appendSlice(read_buf[0..n]);
        if (buf.items.len > max_file_size) break;
    }
    return buf.toOwnedSlice();
}

/// Simple glob pattern matching supporting `*` (any chars) and `?` (single char).
/// Also handles `**` as matching across path separators.
fn matchGlob(pattern: []const u8, name: []const u8) bool {
    return matchGlobImpl(pattern, name, 0, 0);
}

fn matchGlobImpl(pattern: []const u8, name: []const u8, pi: usize, ni: usize) bool {
    var p = pi;
    var n = ni;

    while (p < pattern.len) {
        if (p + 1 < pattern.len and pattern[p] == '*' and pattern[p + 1] == '*') {
            // ** matches everything including path separators
            p += 2;
            // Skip optional trailing slash after **
            if (p < pattern.len and pattern[p] == '/') p += 1;

            // Try matching rest of pattern at every position
            if (p >= pattern.len) return true;
            while (n <= name.len) {
                if (matchGlobImpl(pattern, name, p, n)) return true;
                n += 1;
            }
            return false;
        }

        if (pattern[p] == '*') {
            // * matches anything except path separator
            p += 1;
            if (p >= pattern.len) {
                // * at end: match if no more slashes
                return std.mem.indexOf(u8, name[n..], "/") == null;
            }
            while (n < name.len) {
                if (name[n] == '/') return false;
                if (matchGlobImpl(pattern, name, p, n)) return true;
                n += 1;
            }
            return matchGlobImpl(pattern, name, p, n);
        }

        if (n >= name.len) return false;

        if (pattern[p] == '?') {
            if (name[n] == '/') return false;
            p += 1;
            n += 1;
            continue;
        }

        if (pattern[p] != name[n]) return false;
        p += 1;
        n += 1;
    }

    return n >= name.len;
}

fn errorResult(allocator: Allocator, comptime fmt: []const u8, args: anytype) types.ToolResult {
    const msg = std.fmt.allocPrint(allocator, fmt, args) catch "internal error";
    return .{ .content = msg, .is_error = true };
}

// -- Tests --

test "matchGlob matches simple patterns" {
    try std.testing.expect(matchGlob("*.zig", "main.zig"));
    try std.testing.expect(!matchGlob("*.zig", "main.rs"));
    try std.testing.expect(matchGlob("src/*.zig", "src/main.zig"));
    try std.testing.expect(!matchGlob("src/*.zig", "lib/main.zig"));
}

test "matchGlob handles ** for recursive matching" {
    try std.testing.expect(matchGlob("**/*.zig", "src/main.zig"));
    try std.testing.expect(matchGlob("**/*.zig", "src/llm/anthropic.zig"));
    try std.testing.expect(!matchGlob("**/*.zig", "src/main.rs"));
}

test "matchGlob handles ? for single char" {
    try std.testing.expect(matchGlob("?.zig", "a.zig"));
    try std.testing.expect(!matchGlob("?.zig", "ab.zig"));
}

test "readFile basic" {
    const allocator = std.testing.allocator;
    // Read this test file itself
    const result = try readFile(allocator, "/proc/version", null, @as(?usize, 5));
    defer allocator.free(result.content);
    try std.testing.expect(!result.is_error);
    try std.testing.expect(result.content.len > 0);
}

test "readFile nonexistent" {
    const allocator = std.testing.allocator;
    const result = try readFile(allocator, "/nonexistent/path/file.txt", null, null);
    defer allocator.free(result.content);
    try std.testing.expect(result.is_error);
}
