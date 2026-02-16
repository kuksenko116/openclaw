const std = @import("std");
const Allocator = std.mem.Allocator;
const types = @import("../types.zig");
const streaming = @import("streaming.zig");
const agent_mod = @import("../agent.zig");
/// OpenAI Chat Completions provider.
///
/// Streams responses via SSE from `POST /v1/chat/completions`.
/// Handles the `data: [DONE]` sentinel and accumulates tool call
/// arguments across multiple delta chunks.
pub const OpenAiProvider = struct {
    allocator: Allocator,
    api_key: []const u8,
    model: []const u8,
    base_url: []const u8,

    const default_base_url = "https://api.openai.com";

    pub fn create(
        allocator: Allocator,
        api_key: []const u8,
        model: []const u8,
        base_url: ?[]const u8,
    ) !*OpenAiProvider {
        const self = try allocator.create(OpenAiProvider);
        self.* = .{
            .allocator = allocator,
            .api_key = api_key,
            .model = model,
            .base_url = base_url orelse default_base_url,
        };
        return self;
    }

    pub fn deinit(self: *OpenAiProvider) void {
        self.allocator.destroy(self);
    }

    /// Stream a chat completion. Calls `callback(ctx, event)` for each AgentEvent.
    pub fn streamChat(
        self: *OpenAiProvider,
        allocator: Allocator,
        request: types.ChatRequest,
        ctx: *anyopaque,
        callback: agent_mod.Provider.Callback,
    ) !void {
        const body = try buildRequestBody(allocator, request);
        defer allocator.free(body);

        const url = try std.fmt.allocPrint(allocator, "{s}/v1/chat/completions", .{self.base_url});
        defer allocator.free(url);

        const uri = try std.Uri.parse(url);

        const auth_header = try std.fmt.allocPrint(allocator, "Bearer {s}", .{self.api_key});
        defer allocator.free(auth_header);

        var client = std.http.Client{ .allocator = allocator };
        defer client.deinit();

        var header_buf: [8192]u8 = undefined;
        var req = try client.open(.POST, uri, .{
            .server_header_buffer = &header_buf,
            .extra_headers = &.{
                .{ .name = "content-type", .value = "application/json" },
                .{ .name = "authorization", .value = auth_header },
            },
        });
        defer req.deinit();

        req.transfer_encoding = .{ .content_length = body.len };
        try req.send();
        try req.writeAll(body);
        try req.finish();
        try req.wait();

        if (req.response.status != .ok) {
            return mapHttpError(req.response.status);
        }

        try parseOpenAiStream(allocator, &req, ctx, callback);
    }

    /// Provider vtable glue.
    pub fn streamChatVtable(
        ptr: *anyopaque,
        allocator: Allocator,
        request: types.ChatRequest,
        ctx: *anyopaque,
        callback: agent_mod.Provider.Callback,
    ) anyerror!void {
        const self: *OpenAiProvider = @ptrCast(@alignCast(ptr));
        return self.streamChat(allocator, request, ctx, callback);
    }
};

// -- Request body construction --

fn buildRequestBody(allocator: Allocator, request: types.ChatRequest) ![]const u8 {
    var buf = std.ArrayList(u8).init(allocator);
    errdefer buf.deinit();
    const w = buf.writer();

    try w.writeAll("{");
    try writeStr(w, "model");
    try w.writeAll(":");
    try writeStr(w, request.model);

    try w.writeAll(",");
    try writeStr(w, "max_tokens");
    try w.writeAll(":");
    try w.print("{d}", .{request.max_tokens});

    try w.writeAll(",");
    try writeStr(w, "stream");
    try w.writeAll(":true");

    if (request.temperature) |temp| {
        try w.writeAll(",");
        try writeStr(w, "temperature");
        try w.writeAll(":");
        try w.print("{d:.2}", .{temp});
    }

    // Messages: system prompt as first message
    try w.writeAll(",");
    try writeStr(w, "messages");
    try w.writeAll(":[");

    var msg_idx: usize = 0;
    if (request.system_prompt.len > 0) {
        try w.writeAll("{");
        try writeStr(w, "role");
        try w.writeAll(":");
        try writeStr(w, "system");
        try w.writeAll(",");
        try writeStr(w, "content");
        try w.writeAll(":");
        try writeStr(w, request.system_prompt);
        try w.writeAll("}");
        msg_idx += 1;
    }

    for (request.messages) |msg| {
        if (msg_idx > 0) try w.writeAll(",");
        try writeOpenAiMessage(w, msg);
        msg_idx += 1;
    }
    try w.writeAll("]");

    // Tools
    if (request.tools.len > 0) {
        try w.writeAll(",");
        try writeStr(w, "tools");
        try w.writeAll(":[");
        for (request.tools, 0..) |tool, i| {
            if (i > 0) try w.writeAll(",");
            try w.writeAll("{");
            try writeStr(w, "type");
            try w.writeAll(":");
            try writeStr(w, "function");
            try w.writeAll(",");
            try writeStr(w, "function");
            try w.writeAll(":{");
            try writeStr(w, "name");
            try w.writeAll(":");
            try writeStr(w, tool.name);
            try w.writeAll(",");
            try writeStr(w, "description");
            try w.writeAll(":");
            try writeStr(w, tool.description);
            try w.writeAll(",");
            try writeStr(w, "parameters");
            try w.writeAll(":");
            try w.writeAll(tool.input_schema);
            try w.writeAll("}}");
        }
        try w.writeAll("]");
    }

    try w.writeAll("}");
    return buf.toOwnedSlice();
}

fn writeOpenAiMessage(w: anytype, msg: types.Message) !void {
    // Tool results: each becomes a separate message with role "tool"
    if (msg.role == .user and hasToolResults(msg.content)) {
        var first = true;
        for (msg.content) |block| {
            switch (block) {
                .tool_result => |tr| {
                    if (!first) try w.writeAll(",");
                    try w.writeAll("{");
                    try writeStr(w, "role");
                    try w.writeAll(":");
                    try writeStr(w, "tool");
                    try w.writeAll(",");
                    try writeStr(w, "tool_call_id");
                    try w.writeAll(":");
                    try writeStr(w, tr.tool_use_id);
                    try w.writeAll(",");
                    try writeStr(w, "content");
                    try w.writeAll(":");
                    try writeStr(w, tr.content);
                    try w.writeAll("}");
                    first = false;
                },
                else => {},
            }
        }
        return;
    }

    // Assistant messages with tool calls
    if (msg.role == .assistant and msg.hasToolCalls()) {
        try w.writeAll("{");
        try writeStr(w, "role");
        try w.writeAll(":");
        try writeStr(w, "assistant");
        try w.writeAll(",");
        try writeStr(w, "content");
        try w.writeAll(":");
        const text = msg.textContent();
        if (text.len > 0) {
            try writeStr(w, text);
        } else {
            try w.writeAll("null");
        }

        try w.writeAll(",");
        try writeStr(w, "tool_calls");
        try w.writeAll(":[");
        var tc_idx: usize = 0;
        for (msg.content) |block| {
            switch (block) {
                .tool_use => |tu| {
                    if (tc_idx > 0) try w.writeAll(",");
                    try w.writeAll("{");
                    try writeStr(w, "id");
                    try w.writeAll(":");
                    try writeStr(w, tu.id);
                    try w.writeAll(",");
                    try writeStr(w, "type");
                    try w.writeAll(":");
                    try writeStr(w, "function");
                    try w.writeAll(",");
                    try writeStr(w, "function");
                    try w.writeAll(":{");
                    try writeStr(w, "name");
                    try w.writeAll(":");
                    try writeStr(w, tu.name);
                    try w.writeAll(",");
                    try writeStr(w, "arguments");
                    try w.writeAll(":");
                    try writeStr(w, tu.input);
                    try w.writeAll("}}");
                    tc_idx += 1;
                },
                else => {},
            }
        }
        try w.writeAll("]}");
        return;
    }

    // Regular user/assistant message
    try w.writeAll("{");
    try writeStr(w, "role");
    try w.writeAll(":");
    try writeStr(w, msg.role.toString());
    try w.writeAll(",");
    try writeStr(w, "content");
    try w.writeAll(":");
    try writeStr(w, msg.textContent());
    try w.writeAll("}");
}

fn hasToolResults(blocks: []const types.ContentBlock) bool {
    for (blocks) |block| {
        switch (block) {
            .tool_result => return true,
            else => {},
        }
    }
    return false;
}

// -- SSE stream parsing --

/// Accumulator for a single tool call being built across multiple deltas.
const ToolCallAccum = struct {
    id: std.ArrayList(u8),
    name: std.ArrayList(u8),
    arguments: std.ArrayList(u8),

    fn init(allocator: Allocator) ToolCallAccum {
        return .{
            .id = std.ArrayList(u8).init(allocator),
            .name = std.ArrayList(u8).init(allocator),
            .arguments = std.ArrayList(u8).init(allocator),
        };
    }

    fn deinit(self: *ToolCallAccum) void {
        self.id.deinit();
        self.name.deinit();
        self.arguments.deinit();
    }
};

fn parseOpenAiStream(
    allocator: Allocator,
    req: anytype,
    ctx: *anyopaque,
    callback: agent_mod.Provider.Callback,
) !void {
    streaming.setReadTimeout(req, 300);

    var sse = streaming.SseParser.init(allocator);
    defer sse.deinit();

    var tool_calls = std.ArrayList(ToolCallAccum).init(allocator);
    defer {
        for (tool_calls.items) |*tc| tc.deinit();
        tool_calls.deinit();
    }

    var buf: [4096]u8 = undefined;
    while (true) {
        const n = req.reader().read(&buf) catch |err| {
            if (err == error.WouldBlock) {
                const stderr = std.io.getStdErr().writer();
                stderr.writeAll("\x1b[33mWarning: stream timed out after 300s\x1b[0m\n") catch {};
            }
            break;
        };
        if (n == 0) break;

        const events = try sse.feed(buf[0..n]);
        defer allocator.free(events);

        for (events) |ev| {
            defer allocator.free(ev.data);
            if (ev.event) |et| allocator.free(et);

            if (std.mem.eql(u8, ev.data, "[DONE]")) {
                emitToolCalls(allocator, &tool_calls, ctx, callback);
                callback(ctx, .{ .message_end = .end_turn });
                return;
            }

            processOpenAiChunk(allocator, ev.data, &tool_calls, ctx, callback);
        }
    }
}

fn processOpenAiChunk(
    allocator: Allocator,
    data: []const u8,
    tool_calls: *std.ArrayList(ToolCallAccum),
    ctx: *anyopaque,
    callback: agent_mod.Provider.Callback,
) void {
    const parsed = std.json.parseFromSlice(std.json.Value, allocator, data, .{}) catch return;
    defer parsed.deinit();
    const root = parsed.value;

    // Extract usage from the response (typically in the final chunk)
    if (getObject(root, "usage")) |usage| {
        const input_tokens: u64 = @intCast(@max(getInteger(usage, "prompt_tokens") orelse 0, 0));
        const output_tokens: u64 = @intCast(@max(getInteger(usage, "completion_tokens") orelse 0, 0));
        if (input_tokens > 0 or output_tokens > 0) {
            callback(ctx, .{ .usage_update = .{ .input_tokens = input_tokens, .output_tokens = output_tokens } });
        }
    }

    const choices = getArray(root, "choices") orelse return;
    if (choices.len == 0) return;
    const choice = choices[0];
    const delta = getObject(choice, "delta") orelse return;

    // Check finish_reason
    if (getString(choice, "finish_reason")) |reason| {
        if (std.mem.eql(u8, reason, "tool_calls")) {
            emitToolCalls(allocator, tool_calls, ctx, callback);
            callback(ctx, .{ .message_end = .tool_use });
            return;
        }
        if (std.mem.eql(u8, reason, "stop")) {
            emitToolCalls(allocator, tool_calls, ctx, callback);
            callback(ctx, .{ .message_end = .end_turn });
            return;
        }
        if (std.mem.eql(u8, reason, "length")) {
            callback(ctx, .{ .message_end = .max_tokens });
            return;
        }
    }

    // Text content
    if (getString(delta, "content")) |text| {
        if (text.len > 0) {
            const owned = allocator.dupe(u8, text) catch return;
            callback(ctx, .{ .text_delta = owned });
        }
    }

    // Tool calls delta
    const tc_deltas = getArray(delta, "tool_calls") orelse return;
    for (tc_deltas) |tc_delta| {
        accumulateToolCall(allocator, tc_delta, tool_calls);
    }
}

fn accumulateToolCall(
    allocator: Allocator,
    tc_delta: std.json.Value,
    tool_calls: *std.ArrayList(ToolCallAccum),
) void {
    const index_val = getInteger(tc_delta, "index");
    const index: usize = if (index_val) |v| @intCast(v) else 0;

    while (tool_calls.items.len <= index) {
        tool_calls.append(ToolCallAccum.init(allocator)) catch return;
    }

    var accum = &tool_calls.items[index];

    if (getString(tc_delta, "id")) |id| {
        accum.id.appendSlice(id) catch {};
    }

    if (getObject(tc_delta, "function")) |func| {
        if (getString(func, "name")) |name| {
            accum.name.appendSlice(name) catch {};
        }
        if (getString(func, "arguments")) |args| {
            accum.arguments.appendSlice(args) catch {};
        }
    }
}

fn emitToolCalls(
    allocator: Allocator,
    tool_calls: *std.ArrayList(ToolCallAccum),
    ctx: *anyopaque,
    callback: agent_mod.Provider.Callback,
) void {
    for (tool_calls.items) |*tc| {
        if (tc.name.items.len == 0) continue;

        const id = allocator.dupe(u8, tc.id.items) catch continue;
        const name = allocator.dupe(u8, tc.name.items) catch continue;
        const input = allocator.dupe(u8, tc.arguments.items) catch continue;

        callback(ctx, .{ .tool_use = .{
            .id = id,
            .name = name,
            .input = if (input.len > 0) input else "{}",
        } });
    }
}

// -- JSON helpers --

fn getObject(val: std.json.Value, key: []const u8) ?std.json.Value {
    switch (val) {
        .object => |obj| {
            if (obj.get(key)) |v| {
                switch (v) {
                    .object => return v,
                    else => return null,
                }
            }
            return null;
        },
        else => return null,
    }
}

fn getArray(val: std.json.Value, key: []const u8) ?[]std.json.Value {
    switch (val) {
        .object => |obj| {
            if (obj.get(key)) |v| {
                switch (v) {
                    .array => |a| return a.items,
                    else => return null,
                }
            }
            return null;
        },
        else => return null,
    }
}

fn getString(val: std.json.Value, key: []const u8) ?[]const u8 {
    switch (val) {
        .object => |obj| {
            if (obj.get(key)) |v| {
                switch (v) {
                    .string => |s| return s,
                    else => return null,
                }
            }
            return null;
        },
        else => return null,
    }
}

fn getInteger(val: std.json.Value, key: []const u8) ?i64 {
    switch (val) {
        .object => |obj| {
            if (obj.get(key)) |v| {
                switch (v) {
                    .integer => |i| return i,
                    else => return null,
                }
            }
            return null;
        },
        else => return null,
    }
}

/// Write a JSON-escaped string with surrounding quotes.
fn writeStr(w: anytype, s: []const u8) !void {
    try w.writeByte('"');
    for (s) |c| {
        switch (c) {
            '"' => try w.writeAll("\\\""),
            '\\' => try w.writeAll("\\\\"),
            '\n' => try w.writeAll("\\n"),
            '\r' => try w.writeAll("\\r"),
            '\t' => try w.writeAll("\\t"),
            else => {
                if (c < 0x20) {
                    try w.print("\\u{x:0>4}", .{c});
                } else {
                    try w.writeByte(c);
                }
            },
        }
    }
    try w.writeByte('"');
}

// -- Error mapping --

fn mapHttpError(status: std.http.Status) anyerror {
    return switch (status) {
        .unauthorized => error.AuthenticationFailed,
        .payment_required => error.BillingError,
        .too_many_requests => error.RateLimited,
        .payload_too_large => error.ContextOverflow,
        .bad_request => error.InvalidRequest,
        else => {
            const code: u10 = @intFromEnum(status);
            if (code >= 500) return error.ServerError;
            return error.HttpError;
        },
    };
}

// -- Tests --

test "buildRequestBody includes system prompt as message" {
    const allocator = std.testing.allocator;

    const text_block = types.ContentBlock{ .text = "hi" };
    const blocks = [_]types.ContentBlock{text_block};
    const msg = types.Message{ .role = .user, .content = &blocks };
    const messages = [_]types.Message{msg};

    const request = types.ChatRequest{
        .messages = &messages,
        .system_prompt = "Be helpful.",
        .tools = &.{},
        .model = "gpt-4o",
        .max_tokens = 2048,
        .temperature = null,
    };

    const body = try buildRequestBody(allocator, request);
    defer allocator.free(body);

    const parsed = try std.json.parseFromSlice(std.json.Value, allocator, body, .{});
    defer parsed.deinit();

    const root = parsed.value.object;
    const msgs = root.get("messages").?.array;

    // First message should be system prompt
    const sys_msg = msgs.items[0].object;
    try std.testing.expectEqualStrings("system", sys_msg.get("role").?.string);
    try std.testing.expectEqualStrings("Be helpful.", sys_msg.get("content").?.string);

    // Second message should be the user message
    const user_msg = msgs.items[1].object;
    try std.testing.expectEqualStrings("user", user_msg.get("role").?.string);
}

test "buildRequestBody with tools uses function wrapper" {
    const allocator = std.testing.allocator;

    const text_block = types.ContentBlock{ .text = "list files" };
    const blocks = [_]types.ContentBlock{text_block};
    const msg = types.Message{ .role = .user, .content = &blocks };
    const messages = [_]types.Message{msg};

    const tool_defs = [_]types.ToolDefinition{
        .{
            .name = "bash",
            .description = "Execute a command",
            .input_schema = "{\"type\":\"object\",\"properties\":{\"command\":{\"type\":\"string\"}}}",
        },
    };

    const request = types.ChatRequest{
        .messages = &messages,
        .system_prompt = "",
        .tools = &tool_defs,
        .model = "gpt-4o",
        .max_tokens = 2048,
        .temperature = null,
    };

    const body = try buildRequestBody(allocator, request);
    defer allocator.free(body);

    const parsed = try std.json.parseFromSlice(std.json.Value, allocator, body, .{});
    defer parsed.deinit();

    const root = parsed.value.object;
    const tools = root.get("tools").?.array;
    try std.testing.expectEqual(@as(usize, 1), tools.items.len);

    const tool = tools.items[0].object;
    try std.testing.expectEqualStrings("function", tool.get("type").?.string);

    const func = tool.get("function").?.object;
    try std.testing.expectEqualStrings("bash", func.get("name").?.string);
}
