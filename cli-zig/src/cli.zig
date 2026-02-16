const std = @import("std");
const agent = @import("agent.zig");
const session_mod = @import("session.zig");
const config_mod = @import("config.zig");

const Session = session_mod.Session;
const Config = config_mod.Config;

/// Run the interactive REPL loop.
/// Reads lines from stdin, sends each to the agent loop, and loops
/// until EOF or the user types "exit" or "quit".
pub fn runRepl(
    allocator: std.mem.Allocator,
    session: *Session,
    provider: *const agent.Provider,
    tools: *const agent.ToolRegistry,
    config: *const Config,
) !void {
    const stdin = std.io.getStdIn().reader();
    const stdout = std.io.getStdOut().writer();
    const stderr = std.io.getStdErr().writer();

    try stderr.print(
        "\x1b[1mopenclaw-cli\x1b[0m \x1b[2m({s}:{s})\x1b[0m\n",
        .{ config.provider, config.model },
    );
    try stderr.writeAll("\x1b[2mType /help for commands, \"exit\" or Ctrl+D to quit.\x1b[0m\n\n");

    var line_buf: [8192]u8 = undefined;

    while (true) {
        try stdout.writeAll("\x1b[1;32m❯\x1b[0m ");

        const line = stdin.readUntilDelimiterOrEof(&line_buf, '\n') catch |err| {
            try stderr.print("error reading input: {}\n", .{err});
            continue;
        };

        if (line == null) {
            // EOF
            try stdout.writeByte('\n');
            break;
        }

        const trimmed = std.mem.trim(u8, line.?, " \r\t");
        if (trimmed.len == 0) continue;

        if (std.mem.eql(u8, trimmed, "exit") or std.mem.eql(u8, trimmed, "quit")) {
            break;
        }

        // Handle slash commands
        if (trimmed[0] == '/') {
            try handleSlashCommand(trimmed, config, stderr);
            continue;
        }

        // Add user message and run agent (addUserMessage dupes internally)
        try session.addUserMessage(trimmed);

        _ = agent.runAgentLoop(allocator, provider, session, tools, config) catch |err| {
            try stderr.print("\x1b[31mError: {}\x1b[0m\n", .{err});
            continue;
        };

        try stdout.writeByte('\n');
    }
}

fn handleSlashCommand(
    cmd: []const u8,
    config: *const Config,
    writer: std.fs.File.Writer,
) !void {
    if (std.mem.eql(u8, cmd, "/help") or std.mem.eql(u8, cmd, "/h")) {
        try writer.writeAll(
            \\
            \\  Commands:
            \\    /help, /h      Show this help
            \\    /info          Show current provider and model
            \\    exit, quit     Exit the REPL
            \\    Ctrl+D         Exit (EOF)
            \\
            \\
        );
    } else if (std.mem.eql(u8, cmd, "/info")) {
        try writer.print(
            "  Provider: {s}\n  Model: {s}\n  Max tokens: {d}\n",
            .{ config.provider, config.model, config.max_tokens },
        );
        if (config.base_url) |url| {
            try writer.print("  Base URL: {s}\n", .{url});
        }
    } else {
        try writer.print("\x1b[33mUnknown command: {s}. Type /help for available commands.\x1b[0m\n", .{cmd});
    }
}
