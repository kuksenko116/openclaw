const std = @import("std");
const builtin = @import("builtin");
const config_mod = @import("config.zig");
const session_mod = @import("session.zig");
const agent_mod = @import("agent.zig");
const cli_mod = @import("cli.zig");
const types = @import("types.zig");
const anthropic = @import("llm/anthropic.zig");
const openai = @import("llm/openai.zig");
const ollama = @import("llm/ollama.zig");
const registry_mod = @import("tools/registry.zig");

// -- Argument parsing --

const version = "0.1.0";

const Command = enum {
    chat,
    sessions,
    providers,
    help,
    version,
};

const Args = struct {
    command: Command = .help,
    prompt: ?[]const u8 = null,
    session_name: ?[]const u8 = null,
    interactive: bool = false,
    config_path: ?[]const u8 = null,
    model_override: ?[]const u8 = null,
    provider_override: ?[]const u8 = null,
    api_key_override: ?[]const u8 = null,
    base_url_override: ?[]const u8 = null,
    system_prompt_override: ?[]const u8 = null,
    no_tools: bool = false,
    max_tokens_override: ?u32 = null,
};

fn parseArgs(allocator: std.mem.Allocator) !Args {
    var args = Args{};
    var iter = try std.process.argsWithAllocator(allocator);
    defer iter.deinit();

    // Skip program name
    _ = iter.next();

    // First positional arg is the command
    const cmd_str = iter.next() orelse return args;

    if (std.mem.eql(u8, cmd_str, "chat")) {
        args.command = .chat;
    } else if (std.mem.eql(u8, cmd_str, "sessions")) {
        args.command = .sessions;
    } else if (std.mem.eql(u8, cmd_str, "providers")) {
        args.command = .providers;
    } else if (std.mem.eql(u8, cmd_str, "help") or
        std.mem.eql(u8, cmd_str, "--help") or
        std.mem.eql(u8, cmd_str, "-h"))
    {
        args.command = .help;
        return args;
    } else if (std.mem.eql(u8, cmd_str, "--version") or
        std.mem.eql(u8, cmd_str, "-V"))
    {
        args.command = .version;
        return args;
    } else {
        // Treat unknown first arg as a prompt for chat
        args.command = .chat;
        args.prompt = cmd_str;
        return args;
    }

    // Parse remaining args
    while (iter.next()) |arg| {
        if (std.mem.eql(u8, arg, "-s") or std.mem.eql(u8, arg, "--session")) {
            args.session_name = iter.next();
        } else if (std.mem.eql(u8, arg, "-i") or std.mem.eql(u8, arg, "--interactive")) {
            args.interactive = true;
        } else if (std.mem.eql(u8, arg, "-m") or std.mem.eql(u8, arg, "--model")) {
            args.model_override = iter.next();
        } else if (std.mem.eql(u8, arg, "-p") or std.mem.eql(u8, arg, "--provider")) {
            args.provider_override = iter.next();
        } else if (std.mem.eql(u8, arg, "--api-key")) {
            args.api_key_override = iter.next();
        } else if (std.mem.eql(u8, arg, "--base-url")) {
            args.base_url_override = iter.next();
        } else if (std.mem.eql(u8, arg, "--system-prompt")) {
            args.system_prompt_override = iter.next();
        } else if (std.mem.eql(u8, arg, "--no-tools")) {
            args.no_tools = true;
        } else if (std.mem.eql(u8, arg, "--max-tokens")) {
            if (iter.next()) |val| {
                args.max_tokens_override = std.fmt.parseInt(u32, val, 10) catch null;
            }
        } else if (std.mem.eql(u8, arg, "--config")) {
            args.config_path = iter.next();
        } else if (std.mem.eql(u8, arg, "--help") or std.mem.eql(u8, arg, "-h")) {
            args.command = .help;
            return args;
        } else if (std.mem.eql(u8, arg, "--version") or std.mem.eql(u8, arg, "-V")) {
            args.command = .version;
            return args;
        } else if (!std.mem.startsWith(u8, arg, "-")) {
            // Positional arg: treat as prompt
            if (args.prompt == null) {
                args.prompt = arg;
            }
        } else {
            const stderr = std.io.getStdErr().writer();
            try stderr.print("unknown option: {s}\n", .{arg});
        }
    }

    return args;
}

// -- Provider creation --

fn createProvider(allocator: std.mem.Allocator, config: *const config_mod.Config) !agent_mod.Provider {
    const provider_name = config.provider;
    const api_key = config.api_key orelse "";
    const model = config.model;
    const base_url = config.base_url;

    if (std.mem.eql(u8, provider_name, "anthropic")) {
        const p = try anthropic.AnthropicProvider.create(allocator, api_key, model, base_url);
        return .{
            .ptr = @ptrCast(p),
            .vtable = &.{ .streamChat = &anthropic.AnthropicProvider.streamChatVtable },
        };
    } else if (std.mem.eql(u8, provider_name, "ollama")) {
        const p = try ollama.OllamaProvider.create(allocator, model, base_url);
        return .{
            .ptr = @ptrCast(p),
            .vtable = &.{ .streamChat = &ollama.OllamaProvider.streamChatVtable },
        };
    } else {
        // OpenAI-compatible (openai, openrouter, together, google, gemini, etc.)
        const p = try openai.OpenAiProvider.create(allocator, api_key, model, base_url);
        return .{
            .ptr = @ptrCast(p),
            .vtable = &.{ .streamChat = &openai.OpenAiProvider.streamChatVtable },
        };
    }
}

// -- Command handlers --

fn handleChat(allocator: std.mem.Allocator, args: Args, config: *config_mod.Config) !void {
    const stderr = std.io.getStdErr().writer();

    if (!args.interactive and args.prompt == null) {
        try stderr.writeAll("error: chat requires a prompt or -i for interactive mode\n");
        try printUsage();
        return;
    }

    // Apply CLI overrides (free previously-owned strings before replacing)
    if (args.model_override) |m| {
        if (config.owned.model) allocator.free(config.model);
        config.model = m;
        config.owned.model = false;
    }
    if (args.provider_override) |p| {
        if (config.owned.provider) allocator.free(config.provider);
        config.provider = p;
        config.owned.provider = false;
    }
    if (args.api_key_override) |k| {
        if (config.owned.api_key) if (config.api_key) |old| allocator.free(old);
        config.api_key = k;
        config.owned.api_key = false;
    }
    if (args.base_url_override) |u| {
        if (config.owned.base_url) if (config.base_url) |old| allocator.free(old);
        config.base_url = u;
        config.owned.base_url = false;
    }
    if (args.system_prompt_override) |s| {
        if (config.owned.system_prompt) if (config.system_prompt) |old| allocator.free(old);
        config.system_prompt = s;
        config.owned.system_prompt = false;
    }
    if (args.max_tokens_override) |t| config.max_tokens = t;
    if (args.no_tools) {
        if (config.owned.tools_profile) allocator.free(config.tools_profile);
        config.tools_profile = "none";
        config.owned.tools_profile = false;
    }

    // Validate API key for providers that need one
    if (!std.mem.eql(u8, config.provider, "ollama")) {
        const key = config.api_key orelse "";
        if (key.len == 0) {
            try stderr.print(
                "error: no API key configured for provider '{s}'.\n" ++
                    "Set ANTHROPIC_API_KEY (or OPENAI_API_KEY) env var, " ++
                    "use --api-key, or add api_key to config.\n",
                .{config.provider},
            );
            return;
        }
    }

    // Resolve session path
    const sessions_dir = if (config.sessions_dir) |d|
        try allocator.dupe(u8, d)
    else
        try config_mod.defaultSessionsDir(allocator);
    defer allocator.free(sessions_dir);

    const session_name = args.session_name orelse "default";
    const session_path = try session_mod.sessionPath(allocator, sessions_dir, session_name);
    defer allocator.free(session_path);

    var session = try session_mod.Session.load(allocator, session_path);
    defer session.deinit();

    // Create provider and tool registry
    const provider = try createProvider(allocator, config);

    const tools_profile = if (args.no_tools) "none" else config.tools_profile;
    var tool_reg = registry_mod.ToolRegistry.init(
        allocator,
        tools_profile,
        config.exec_security,
        config.exec_allowlist,
    );
    const tools = agent_mod.ToolRegistry{
        .ptr = @ptrCast(&tool_reg),
        .vtable = &.{
            .execute = &registry_mod.ToolRegistry.executeVtable,
            .definitions = &registry_mod.ToolRegistry.definitionsVtable,
        },
    };

    if (args.interactive) {
        try cli_mod.runRepl(allocator, &session, &provider, &tools, config);
    } else if (args.prompt) |prompt| {
        try session.addUserMessage(prompt);
        const result = try agent_mod.runAgentLoop(allocator, &provider, &session, &tools, config);

        // Print usage stats
        if (result.tool_calls > 0 or result.usage.input_tokens > 0) {
            try stderr.print(
                "\n\x1b[2m({d} tool call{s}, {d} in / {d} out tokens)\x1b[0m\n",
                .{
                    result.tool_calls,
                    @as([]const u8, if (result.tool_calls == 1) "" else "s"),
                    result.usage.input_tokens,
                    result.usage.output_tokens,
                },
            );
        }
    }
}

fn handleSessions(allocator: std.mem.Allocator, config: *const config_mod.Config) !void {
    const stdout = std.io.getStdOut().writer();

    const sessions_dir = if (config.sessions_dir) |d|
        try allocator.dupe(u8, d)
    else
        try config_mod.defaultSessionsDir(allocator);
    defer allocator.free(sessions_dir);

    var dir = std.fs.openDirAbsolute(sessions_dir, .{ .iterate = true }) catch |err| {
        if (err == error.FileNotFound) {
            try stdout.writeAll("no sessions found\n");
            return;
        }
        return err;
    };
    defer dir.close();

    var count: usize = 0;
    var iter = dir.iterate();
    while (try iter.next()) |entry| {
        if (entry.kind != .file) continue;
        if (!std.mem.endsWith(u8, entry.name, ".jsonl")) continue;

        const name = entry.name[0 .. entry.name.len - 6]; // strip .jsonl
        try stdout.print("{s}\n", .{name});
        count += 1;
    }

    if (count == 0) {
        try stdout.writeAll("no sessions found\n");
    }
}

fn handleProviders(config: *const config_mod.Config) !void {
    const stdout = std.io.getStdOut().writer();
    try stdout.print("Configured provider: {s}\n", .{config.provider});
    try stdout.print("Model: {s}\n", .{config.model});
    if (config.base_url) |url| {
        try stdout.print("Base URL: {s}\n", .{url});
    }
    const has_key = if (config.api_key) |k| k.len > 0 else false;
    try stdout.print("API key: {s}\n", .{if (has_key) "set" else "not set"});
}

fn printUsage() !void {
    const stdout = std.io.getStdOut().writer();
    try stdout.writeAll(
        \\usage: openclaw-cli <command> [options]
        \\
        \\commands:
        \\  chat <prompt>            send a prompt to the AI
        \\  sessions                 list saved sessions
        \\  providers                show provider configuration
        \\  help                     show this help
        \\
        \\chat options:
        \\  -s, --session <name>     use a named session (default: "default")
        \\  -i, --interactive        start interactive REPL mode
        \\  -m, --model <model>      override the model
        \\  -p, --provider <name>    override the provider (anthropic, openai, ollama)
        \\  --api-key <key>          override the API key
        \\  --base-url <url>         override the provider base URL
        \\  --system-prompt <text>   override the system prompt
        \\  --no-tools               disable tool use
        \\  --max-tokens <n>         max tokens for the response
        \\  --config <path>          path to config file
        \\
        \\global options:
        \\  -h, --help               show this help
        \\  -V, --version            show version
        \\
    );
}

// -- Entry point --

pub fn main() !void {
    var gpa = std.heap.GeneralPurposeAllocator(.{
        .stack_trace_frames = if (builtin.mode == .Debug) 8 else 0,
    }){};
    defer {
        const check = gpa.deinit();
        if (check == .leak) {
            std.log.err("memory leak detected", .{});
        }
    }
    const allocator = gpa.allocator();

    const args = parseArgs(allocator) catch |err| {
        const stderr = std.io.getStdErr().writer();
        try stderr.print("error parsing arguments: {}\n", .{err});
        try printUsage();
        return;
    };

    // Load config
    const config_path = if (args.config_path) |p|
        try allocator.dupe(u8, p)
    else
        config_mod.defaultConfigPath(allocator) catch |err| {
            const stderr = std.io.getStdErr().writer();
            try stderr.print("error resolving config path: {}\n", .{err});
            return;
        };
    defer allocator.free(config_path);

    var config = config_mod.load(allocator, config_path) catch |err| {
        const stderr = std.io.getStdErr().writer();
        try stderr.print("error loading config: {}\n", .{err});
        return;
    };
    defer config.deinit(allocator);

    // Validate config
    config_mod.validate(&config) catch {
        const stderr = std.io.getStdErr().writer();
        try stderr.writeAll("error: invalid configuration (check provider, model, temperature, max_tokens)\n");
        return;
    };

    // Dispatch command
    switch (args.command) {
        .chat => try handleChat(allocator, args, &config),
        .sessions => try handleSessions(allocator, &config),
        .providers => try handleProviders(&config),
        .help => try printUsage(),
        .version => {
            const stdout = std.io.getStdOut().writer();
            try stdout.print("openclaw-cli {s}\n", .{version});
        },
    }
}

// -- Tests --

// Pull in tests from all modules so `zig build test` runs them.
comptime {
    _ = @import("types.zig");
    _ = @import("config.zig");
    _ = @import("session.zig");
    _ = @import("agent.zig");
    _ = @import("cli.zig");
    _ = @import("llm/streaming.zig");
    _ = @import("llm/anthropic.zig");
    _ = @import("llm/openai.zig");
    _ = @import("llm/ollama.zig");
}

test "parseArgs returns help for no args" {
    // NOTE: We cannot easily test parseArgs directly because it reads from
    // std.process.argsWithAllocator which returns the real process args.
    // Instead, we verify the default struct.
    const args = Args{};
    try std.testing.expectEqual(Command.help, args.command);
    try std.testing.expect(args.prompt == null);
    try std.testing.expect(!args.interactive);
}
