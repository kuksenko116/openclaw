const std = @import("std");
const types = @import("types.zig");
const session_mod = @import("session.zig");
const config_mod = @import("config.zig");

const Message = types.Message;
const ContentBlock = types.ContentBlock;
const AgentEvent = types.AgentEvent;
const ChatRequest = types.ChatRequest;
const ToolDefinition = types.ToolDefinition;
const ToolResult = types.ToolResult;
const Usage = types.Usage;
const StopReason = types.StopReason;
const Session = session_mod.Session;
const Config = config_mod.Config;

/// Result returned from a completed agent run.
pub const AgentResult = struct {
    text: []const u8,
    usage: Usage,
    iterations: u32,
    tool_calls: u32 = 0,
};

/// Provider interface. Implemented via vtable for runtime polymorphism.
pub const Provider = struct {
    ptr: *anyopaque,
    vtable: *const VTable,

    /// Callback function type: receives a context pointer and an event.
    /// This is the standard Zig pattern for callbacks (no closures).
    pub const Callback = *const fn (ctx: *anyopaque, event: AgentEvent) void;

    const VTable = struct {
        streamChat: *const fn (
            ptr: *anyopaque,
            allocator: std.mem.Allocator,
            request: ChatRequest,
            ctx: *anyopaque,
            callback: Callback,
        ) anyerror!void,
    };

    pub fn streamChat(
        self: Provider,
        allocator: std.mem.Allocator,
        request: ChatRequest,
        ctx: *anyopaque,
        callback: Callback,
    ) !void {
        return self.vtable.streamChat(self.ptr, allocator, request, ctx, callback);
    }
};

/// Tool registry interface. Maps tool names to execution functions.
pub const ToolRegistry = struct {
    ptr: *anyopaque,
    vtable: *const VTable,

    const VTable = struct {
        execute: *const fn (ptr: *anyopaque, name: []const u8, input: []const u8) anyerror!ToolResult,
        definitions: *const fn (ptr: *anyopaque) []const ToolDefinition,
    };

    pub fn execute(self: ToolRegistry, name: []const u8, input: []const u8) !ToolResult {
        return self.vtable.execute(self.ptr, name, input);
    }

    pub fn definitions(self: ToolRegistry) []const ToolDefinition {
        return self.vtable.definitions(self.ptr);
    }
};

const max_iterations = 20;
const max_retries = 2;

/// IO writers used by the agent loop. Defaults to stdout/stderr.
pub const AgentIO = struct {
    out: std.io.AnyWriter,
    err: std.io.AnyWriter,

    pub fn default() AgentIO {
        return .{
            .out = std.io.getStdOut().writer().any(),
            .err = std.io.getStdErr().writer().any(),
        };
    }
};

/// Run the agent loop: send messages to the provider, handle tool calls, repeat.
/// Uses stdout/stderr for output.
pub fn runAgentLoop(
    allocator: std.mem.Allocator,
    provider: *const Provider,
    session: *Session,
    tools: *const ToolRegistry,
    config: *const Config,
) !AgentResult {
    return runAgentLoopWithIO(allocator, provider, session, tools, config, AgentIO.default());
}

/// Run the agent loop with explicit IO writers (for testing).
pub fn runAgentLoopWithIO(
    allocator: std.mem.Allocator,
    provider: *const Provider,
    session: *Session,
    tools: *const ToolRegistry,
    config: *const Config,
    io: AgentIO,
) !AgentResult {
    var total_usage = Usage{};
    var iteration: u32 = 0;
    var total_tool_calls: u32 = 0;

    while (iteration < max_iterations) : (iteration += 1) {
        const messages = session.getMessages();

        const request = ChatRequest{
            .messages = messages,
            .system_prompt = config.system_prompt orelse "You are a helpful assistant.",
            .tools = tools.definitions(),
            .model = config.model,
            .max_tokens = config.max_tokens,
            .temperature = config.temperature,
        };

        // Collect events from streaming
        var response_text = std.ArrayList(u8).init(allocator);
        defer response_text.deinit();

        var tool_calls = std.ArrayList(types.ToolUse).init(allocator);
        defer tool_calls.deinit();

        var stop_reason: StopReason = .end_turn;

        // Callback context: captures mutable state for the event callback.
        // Zig doesn't have closures, so we pass a context pointer alongside
        // the function pointer — the standard Zig callback pattern.
        const CallbackContext = struct {
            text: *std.ArrayList(u8),
            tool_calls_list: *std.ArrayList(types.ToolUse),
            reason: *StopReason,
            usage: *Usage,
            out: std.io.AnyWriter,
            err_out: std.io.AnyWriter,

            fn callback(ctx_ptr: *anyopaque, event: AgentEvent) void {
                const self: *@This() = @ptrCast(@alignCast(ctx_ptr));
                switch (event) {
                    .text_delta => |text| {
                        self.out.writeAll(text) catch {};
                        self.text.appendSlice(text) catch {};
                    },
                    .tool_use => |tu| {
                        self.err_out.print(
                            "\n\x1b[36m⚙ Tool: {s}\x1b[0m \x1b[2m({s})\x1b[0m\n",
                            .{ tu.name, tu.id },
                        ) catch {};
                        self.tool_calls_list.append(tu) catch {};
                    },
                    .message_end => |reason| {
                        self.reason.* = reason;
                    },
                    .usage_update => |u| {
                        self.usage.input_tokens += u.input_tokens;
                        self.usage.output_tokens += u.output_tokens;
                    },
                }
            }
        };

        var ctx = CallbackContext{
            .text = &response_text,
            .tool_calls_list = &tool_calls,
            .reason = &stop_reason,
            .usage = &total_usage,
            .out = io.out,
            .err_out = io.err,
        };

        // Retry loop for transient errors (rate limit, server overload)
        var last_err: ?anyerror = null;
        for (0..max_retries + 1) |attempt| {
            provider.streamChat(allocator, request, @ptrCast(&ctx), &CallbackContext.callback) catch |err| {
                if (attempt < max_retries and isRetryableError(err)) {
                    const delay_secs: u64 = @as(u64, 1) << @intCast(attempt);
                    io.err.print(
                        "\x1b[33mRetryable error (attempt {d}/{d}): {s}. Retrying in {d}s…\x1b[0m\n",
                        .{ attempt + 1, max_retries, @errorName(err), delay_secs },
                    ) catch {};
                    std.time.sleep(delay_secs * std.time.ns_per_s);
                    last_err = err;
                    continue;
                }
                io.err.print("error: provider call failed: {}\n", .{err}) catch {};
                return err;
            };
            last_err = null;
            break;
        }
        if (last_err) |err| return err;

        // Newline after streamed text
        if (response_text.items.len > 0) {
            io.out.writeByte('\n') catch {};
        }

        // Build the assistant message with all content blocks (text + tool uses)
        var content_blocks = std.ArrayList(ContentBlock).init(allocator);
        defer content_blocks.deinit();

        if (response_text.items.len > 0) {
            try content_blocks.append(.{ .text = try allocator.dupe(u8, response_text.items) });
        }
        for (tool_calls.items) |tc| {
            try content_blocks.append(.{ .tool_use = .{
                .id = try allocator.dupe(u8, tc.id),
                .name = try allocator.dupe(u8, tc.name),
                .input = try allocator.dupe(u8, tc.input),
            } });
        }
        if (content_blocks.items.len > 0) {
            try session.addAssistantMessage(try content_blocks.toOwnedSlice());
        }

        // Handle tool calls
        if (stop_reason == .tool_use and tool_calls.items.len > 0) {
            for (tool_calls.items) |tc| {
                total_tool_calls += 1;
                io.err.print("\x1b[2m  Running {s}…\x1b[0m\n", .{tc.name}) catch {};

                const result = tools.execute(tc.name, tc.input) catch |err| {
                    io.err.print("\x1b[31m  ✗ Error: {}\x1b[0m\n", .{err}) catch {};
                    const err_msg = try std.fmt.allocPrint(allocator, "tool error: {}", .{err});
                    try addToolResult(allocator, session, tc.id, err_msg, true);
                    continue;
                };

                // Print a truncated preview of the result
                const preview = if (result.content.len > 200) result.content[0..200] else result.content;
                if (result.is_error) {
                    io.err.print("\x1b[31m  ✗ {s}\x1b[0m\n", .{preview}) catch {};
                } else {
                    io.err.print("\x1b[32m  ✓\x1b[0m \x1b[2m{s}\x1b[0m\n", .{preview}) catch {};
                }

                try addToolResult(allocator, session, tc.id, result.content, result.is_error);
            }
            continue; // Loop back to send tool results
        }

        // Done
        try session.save();
        return .{
            .text = try allocator.dupe(u8, response_text.items),
            .usage = total_usage,
            .iterations = iteration + 1,
            .tool_calls = total_tool_calls,
        };
    }

    return error.MaxIterationsReached;
}

/// Check if an error is retryable (rate limits, server overload).
fn isRetryableError(err: anyerror) bool {
    return err == error.RateLimited or
        err == error.ServerError or
        err == error.ConnectionRefused or
        err == error.ConnectionResetByPeer;
}

fn addToolResult(
    allocator: std.mem.Allocator,
    session: *Session,
    tool_use_id: []const u8,
    content: []const u8,
    is_error: bool,
) !void {
    const blocks = try allocator.alloc(ContentBlock, 1);
    blocks[0] = .{ .tool_result = .{
        .tool_use_id = try allocator.dupe(u8, tool_use_id),
        .content = try allocator.dupe(u8, content),
        .is_error = is_error,
    } };

    try session.messages.append(.{
        .role = .user,
        .content = blocks,
    });
    session.dirty = true;
}

// -- Tests --

test "AgentResult has expected fields" {
    const result = AgentResult{
        .text = "hello",
        .usage = .{ .input_tokens = 10, .output_tokens = 20 },
        .iterations = 1,
    };
    try std.testing.expectEqualStrings("hello", result.text);
    try std.testing.expectEqual(@as(u64, 10), result.usage.input_tokens);
    try std.testing.expectEqual(@as(u32, 1), result.iterations);
}

// -- Mock provider for testing --

const MockProvider = struct {
    /// Canned responses: each entry is a list of events for one turn.
    turns: []const []const AgentEvent,
    turn_index: usize = 0,

    fn streamChatImpl(ptr: *anyopaque, _: std.mem.Allocator, _: ChatRequest, ctx: *anyopaque, callback: Provider.Callback) anyerror!void {
        const self: *MockProvider = @ptrCast(@alignCast(ptr));
        if (self.turn_index >= self.turns.len) return error.NoMoreTurns;
        const events = self.turns[self.turn_index];
        self.turn_index += 1;
        for (events) |event| {
            callback(ctx, event);
        }
    }

    fn asProvider(self: *MockProvider) Provider {
        return .{
            .ptr = @ptrCast(self),
            .vtable = &.{ .streamChat = &streamChatImpl },
        };
    }
};

const MockTools = struct {
    fn executeImpl(_: *anyopaque, _: []const u8, _: []const u8) anyerror!ToolResult {
        return .{ .content = "mock result", .is_error = false };
    }

    fn definitionsImpl(_: *anyopaque) []const ToolDefinition {
        return &.{};
    }

    fn asRegistry(self: *MockTools) ToolRegistry {
        return .{
            .ptr = @ptrCast(self),
            .vtable = &.{
                .execute = &executeImpl,
                .definitions = &definitionsImpl,
            },
        };
    }
};

/// Create a null writer that discards all output (for tests).
fn nullWrite(_: *const anyopaque, data: []const u8) error{}!usize {
    return data.len;
}

fn testIO() AgentIO {
    const null_writer = std.io.AnyWriter{
        .context = @ptrFromInt(1), // non-null dummy
        .writeFn = &nullWrite,
    };
    return .{ .out = null_writer, .err = null_writer };
}

test "runAgentLoop text-only response" {
    const allocator = std.testing.allocator;

    // Mock provider: returns a text response then ends
    const turn1 = [_]AgentEvent{
        .{ .text_delta = "Hello " },
        .{ .text_delta = "world!" },
        .{ .usage_update = .{ .input_tokens = 50, .output_tokens = 10 } },
        .{ .message_end = .end_turn },
    };
    const turns = [_][]const AgentEvent{&turn1};
    var mock_provider = MockProvider{ .turns = &turns };
    var provider = mock_provider.asProvider();

    var mock_tools = MockTools{};
    var tools = mock_tools.asRegistry();

    const config = Config{};

    // Session with a /tmp path so save() works
    var session = Session.init(allocator, "test", "/tmp/zig-agent-test.jsonl");
    defer session.deinit();
    try session.addUserMessage("hello");

    const result = try runAgentLoopWithIO(allocator, &provider, &session, &tools, &config, testIO());
    defer allocator.free(result.text);

    try std.testing.expectEqualStrings("Hello world!", result.text);
    try std.testing.expectEqual(@as(u32, 0), result.tool_calls);
    try std.testing.expectEqual(@as(u32, 1), result.iterations);
    try std.testing.expectEqual(@as(u64, 50), result.usage.input_tokens);
    try std.testing.expectEqual(@as(u64, 10), result.usage.output_tokens);
    // Session should have: user message + assistant message
    try std.testing.expectEqual(@as(usize, 2), session.messages.items.len);

    // Clean up temp file
    std.fs.deleteFileAbsolute("/tmp/zig-agent-test.jsonl") catch {};
}

test "runAgentLoop with tool call" {
    const allocator = std.testing.allocator;

    // Turn 1: LLM calls a tool
    const turn1 = [_]AgentEvent{
        .{ .tool_use = .{ .id = "t1", .name = "bash", .input = "{\"command\":\"echo hi\"}" } },
        .{ .message_end = .tool_use },
    };
    // Turn 2: LLM responds with text after seeing tool result
    const turn2 = [_]AgentEvent{
        .{ .text_delta = "Done!" },
        .{ .message_end = .end_turn },
    };
    const turns = [_][]const AgentEvent{ &turn1, &turn2 };
    var mock_provider = MockProvider{ .turns = &turns };
    var provider = mock_provider.asProvider();

    var mock_tools = MockTools{};
    var tools = mock_tools.asRegistry();

    const config = Config{};

    var session = Session.init(allocator, "test", "/tmp/zig-agent-test2.jsonl");
    defer session.deinit();
    try session.addUserMessage("run echo hi");

    const result = try runAgentLoopWithIO(allocator, &provider, &session, &tools, &config, testIO());
    defer allocator.free(result.text);

    try std.testing.expectEqualStrings("Done!", result.text);
    try std.testing.expectEqual(@as(u32, 1), result.tool_calls);
    try std.testing.expectEqual(@as(u32, 2), result.iterations);
    // Session: user, assistant(tool_use), user(tool_result), assistant(text)
    try std.testing.expectEqual(@as(usize, 4), session.messages.items.len);

    // Clean up temp file
    std.fs.deleteFileAbsolute("/tmp/zig-agent-test2.jsonl") catch {};
}

test "isRetryableError identifies retryable errors" {
    try std.testing.expect(isRetryableError(error.RateLimited));
    try std.testing.expect(isRetryableError(error.ServerError));
    try std.testing.expect(isRetryableError(error.ConnectionRefused));
    try std.testing.expect(isRetryableError(error.ConnectionResetByPeer));
    try std.testing.expect(!isRetryableError(error.AuthenticationFailed));
    try std.testing.expect(!isRetryableError(error.InvalidRequest));
    try std.testing.expect(!isRetryableError(error.NoMoreTurns));
}

test "mock provider delivers events via callback" {
    const turn1 = [_]AgentEvent{
        .{ .text_delta = "Hello " },
        .{ .text_delta = "world!" },
        .{ .message_end = .end_turn },
    };
    const turns = [_][]const AgentEvent{&turn1};

    var mock_provider = MockProvider{ .turns = &turns };
    const provider = mock_provider.asProvider();

    const CollectCtx = struct {
        count: usize = 0,
        fn cb(ctx_ptr: *anyopaque, _: AgentEvent) void {
            const self: *@This() = @ptrCast(@alignCast(ctx_ptr));
            self.count += 1;
        }
    };

    var ctx = CollectCtx{};
    try provider.streamChat(
        std.testing.allocator,
        .{
            .messages = &.{},
            .system_prompt = "",
            .tools = &.{},
            .model = "test",
            .max_tokens = 100,
            .temperature = null,
        },
        @ptrCast(&ctx),
        &CollectCtx.cb,
    );

    try std.testing.expectEqual(@as(usize, 3), ctx.count);
}

test "mock provider advances turns" {
    const turn1 = [_]AgentEvent{.{ .message_end = .end_turn }};
    const turn2 = [_]AgentEvent{.{ .text_delta = "second" }, .{ .message_end = .end_turn }};
    const turns = [_][]const AgentEvent{ &turn1, &turn2 };

    var mock_provider = MockProvider{ .turns = &turns };
    const provider = mock_provider.asProvider();

    const NoopCtx = struct {
        fn cb(_: *anyopaque, _: AgentEvent) void {}
    };

    var ctx: u8 = 0;
    const req = ChatRequest{
        .messages = &.{},
        .system_prompt = "",
        .tools = &.{},
        .model = "test",
        .max_tokens = 100,
        .temperature = null,
    };

    try provider.streamChat(std.testing.allocator, req, @ptrCast(&ctx), &NoopCtx.cb);
    try std.testing.expectEqual(@as(usize, 1), mock_provider.turn_index);

    try provider.streamChat(std.testing.allocator, req, @ptrCast(&ctx), &NoopCtx.cb);
    try std.testing.expectEqual(@as(usize, 2), mock_provider.turn_index);

    // Third call should error - no more turns
    const err = provider.streamChat(std.testing.allocator, req, @ptrCast(&ctx), &NoopCtx.cb);
    try std.testing.expectError(error.NoMoreTurns, err);
}
