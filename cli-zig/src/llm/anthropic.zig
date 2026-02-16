const std = @import("std");
const Allocator = std.mem.Allocator;
const types = @import("../types.zig");
const streaming = @import("streaming.zig");
const agent_mod = @import("../agent.zig");

/// Anthropic Messages API provider.
///
/// Streams responses via SSE from `POST /v1/messages`. Parses Anthropic
/// event types (content_block_delta, content_block_start, message_delta)
/// and emits unified AgentEvent values.
pub const AnthropicProvider = struct {
    allocator: Allocator,
    api_key: []const u8,
    model: []const u8,
    base_url: []const u8,

    const default_base_url = "https://api.anthropic.com";
    const api_version = "2023-06-01";

    pub fn create(
        allocator: Allocator,
        api_key: []const u8,
        model: []const u8,
        base_url: ?[]const u8,
    ) !*AnthropicProvider {
        const self = try allocator.create(AnthropicProvider);
        self.* = .{
            .allocator = allocator,
            .api_key = api_key,
            .model = model,
            .base_url = base_url orelse default_base_url,
        };
        return self;
    }

    pub fn deinit(self: *AnthropicProvider) void {
        self.allocator.destroy(self);
    }

    /// Stream a chat completion. Calls `callback(ctx, event)` for each AgentEvent.
    pub fn streamChat(
        self: *AnthropicProvider,
        allocator: Allocator,
        request: types.ChatRequest,
        ctx: *anyopaque,
        callback: agent_mod.Provider.Callback,
    ) !void {
        const body = try buildRequestBody(allocator, request);
        defer allocator.free(body);

        const url = try std.fmt.allocPrint(allocator, "{s}/v1/messages", .{self.base_url});
        defer allocator.free(url);

        const uri = try std.Uri.parse(url);

        var client = std.http.Client{ .allocator = allocator };
        defer client.deinit();

        var header_buf: [8192]u8 = undefined;
        var req = try client.open(.POST, uri, .{
            .server_header_buffer = &header_buf,
            .extra_headers = &.{
                .{ .name = "content-type", .value = "application/json" },
                .{ .name = "x-api-key", .value = self.api_key },
                .{ .name = "anthropic-version", .value = api_version },
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

        try parseAnthropicStream(allocator, &req, ctx, callback);
    }

    /// Provider vtable glue: cast from opaque pointer and dispatch.
    pub fn streamChatVtable(
        ptr: *anyopaque,
        allocator: Allocator,
        request: types.ChatRequest,
        ctx: *anyopaque,
        callback: agent_mod.Provider.Callback,
    ) anyerror!void {
        const self: *AnthropicProvider = @ptrCast(@alignCast(ptr));
        return self.streamChat(allocator, request, ctx, callback);
    }
};

// -- Request body construction (manual JSON building) --

fn buildRequestBody(allocator: Allocator, request: types.ChatRequest) ![]const u8 {
    var buf = std.ArrayList(u8).init(allocator);
    errdefer buf.deinit();
    const w = buf.writer();

    try w.writeAll("{");
    try writeJsonString(w, "model");
    try w.writeAll(":");
    try writeJsonString(w, request.model);

    try w.writeAll(",");
    try writeJsonString(w, "max_tokens");
    try w.writeAll(":");
    try w.print("{d}", .{request.max_tokens});

    try w.writeAll(",");
    try writeJsonString(w, "stream");
    try w.writeAll(":true");

    if (request.system_prompt.len > 0) {
        try w.writeAll(",");
        try writeJsonString(w, "system");
        try w.writeAll(":");
        try writeJsonString(w, request.system_prompt);
    }

    if (request.temperature) |temp| {
        try w.writeAll(",");
        try writeJsonString(w, "temperature");
        try w.writeAll(":");
        try w.print("{d:.2}", .{temp});
    }

    try w.writeAll(",");
    try writeJsonString(w, "messages");
    try w.writeAll(":");
    try writeMessages(w, request.messages);

    if (request.tools.len > 0) {
        try w.writeAll(",");
        try writeJsonString(w, "tools");
        try w.writeAll(":");
        try writeTools(w, request.tools);
    }

    try w.writeAll("}");

    return buf.toOwnedSlice();
}

fn writeMessages(w: anytype, messages: []const types.Message) !void {
    try w.writeAll("[");
    for (messages, 0..) |msg, i| {
        if (i > 0) try w.writeAll(",");
        try writeMessage(w, msg);
    }
    try w.writeAll("]");
}

fn writeMessage(w: anytype, msg: types.Message) !void {
    try w.writeAll("{");
    try writeJsonString(w, "role");
    try w.writeAll(":");
    try writeJsonString(w, msg.role.toString());
    try w.writeAll(",");
    try writeJsonString(w, "content");
    try w.writeAll(":");
    try writeContentBlocks(w, msg.content);
    try w.writeAll("}");
}

fn writeContentBlocks(w: anytype, blocks: []const types.ContentBlock) !void {
    // Single text block: write as plain string (Anthropic shorthand)
    if (blocks.len == 1) {
        switch (blocks[0]) {
            .text => |t| {
                try writeJsonString(w, t);
                return;
            },
            else => {},
        }
    }

    try w.writeAll("[");
    for (blocks, 0..) |block, i| {
        if (i > 0) try w.writeAll(",");
        switch (block) {
            .text => |t| {
                try w.writeAll("{");
                try writeJsonString(w, "type");
                try w.writeAll(":");
                try writeJsonString(w, "text");
                try w.writeAll(",");
                try writeJsonString(w, "text");
                try w.writeAll(":");
                try writeJsonString(w, t);
                try w.writeAll("}");
            },
            .tool_use => |tu| {
                try w.writeAll("{");
                try writeJsonString(w, "type");
                try w.writeAll(":");
                try writeJsonString(w, "tool_use");
                try w.writeAll(",");
                try writeJsonString(w, "id");
                try w.writeAll(":");
                try writeJsonString(w, tu.id);
                try w.writeAll(",");
                try writeJsonString(w, "name");
                try w.writeAll(":");
                try writeJsonString(w, tu.name);
                try w.writeAll(",");
                try writeJsonString(w, "input");
                try w.writeAll(":");
                // input is raw JSON
                try w.writeAll(tu.input);
                try w.writeAll("}");
            },
            .tool_result => |tr| {
                try w.writeAll("{");
                try writeJsonString(w, "type");
                try w.writeAll(":");
                try writeJsonString(w, "tool_result");
                try w.writeAll(",");
                try writeJsonString(w, "tool_use_id");
                try w.writeAll(":");
                try writeJsonString(w, tr.tool_use_id);
                try w.writeAll(",");
                try writeJsonString(w, "content");
                try w.writeAll(":");
                try writeJsonString(w, tr.content);
                if (tr.is_error) {
                    try w.writeAll(",");
                    try writeJsonString(w, "is_error");
                    try w.writeAll(":true");
                }
                try w.writeAll("}");
            },
        }
    }
    try w.writeAll("]");
}

fn writeTools(w: anytype, tools: []const types.ToolDefinition) !void {
    try w.writeAll("[");
    for (tools, 0..) |tool, i| {
        if (i > 0) try w.writeAll(",");
        try w.writeAll("{");
        try writeJsonString(w, "name");
        try w.writeAll(":");
        try writeJsonString(w, tool.name);
        try w.writeAll(",");
        try writeJsonString(w, "description");
        try w.writeAll(":");
        try writeJsonString(w, tool.description);
        try w.writeAll(",");
        try writeJsonString(w, "input_schema");
        try w.writeAll(":");
        // input_schema is pre-serialized JSON
        try w.writeAll(tool.input_schema);
        try w.writeAll("}");
    }
    try w.writeAll("]");
}

// -- SSE stream parsing --

const StreamState = struct {
    allocator: Allocator,
    ctx: *anyopaque,
    callback: agent_mod.Provider.Callback,
    current_tool_id: ?[]const u8 = null,
    current_tool_name: ?[]const u8 = null,
    tool_input_buf: std.ArrayList(u8),
    in_tool_use: bool = false,

    fn init(allocator: Allocator, ctx: *anyopaque, callback: agent_mod.Provider.Callback) StreamState {
        return .{
            .allocator = allocator,
            .ctx = ctx,
            .callback = callback,
            .tool_input_buf = std.ArrayList(u8).init(allocator),
        };
    }

    fn deinit(self: *StreamState) void {
        self.tool_input_buf.deinit();
        if (self.current_tool_id) |id| self.allocator.free(id);
        if (self.current_tool_name) |name| self.allocator.free(name);
    }

    /// Emit accumulated tool_use and reset state.
    fn emitToolUse(self: *StreamState) void {
        if (!self.in_tool_use) return;

        const input = self.tool_input_buf.toOwnedSlice() catch return;
        self.callback(self.ctx, .{ .tool_use = .{
            .id = self.current_tool_id orelse "unknown",
            .name = self.current_tool_name orelse "unknown",
            .input = if (input.len > 0) input else "{}",
        } });

        self.current_tool_id = null;
        self.current_tool_name = null;
        self.in_tool_use = false;
    }
};

fn parseAnthropicStream(
    allocator: Allocator,
    req: anytype,
    ctx: *anyopaque,
    callback: agent_mod.Provider.Callback,
) !void {
    streaming.setReadTimeout(req, 300);

    var state = StreamState.init(allocator, ctx, callback);
    defer state.deinit();

    var sse = streaming.SseParser.init(allocator);
    defer sse.deinit();

    var buf: [4096]u8 = undefined;
    while (true) {
        const n = req.reader().read(&buf) catch |err| {
            // EAGAIN/ETIMEDOUT from SO_RCVTIMEO → treat as stream timeout
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
            if (ev.event) |et| {
                defer allocator.free(et);
                processAnthropicEvent(et, ev.data, &state);
            }
        }
    }
}

fn processAnthropicEvent(
    event_type: []const u8,
    data: []const u8,
    state: *StreamState,
) void {
    const parsed = std.json.parseFromSlice(std.json.Value, state.allocator, data, .{}) catch return;
    defer parsed.deinit();
    const root = parsed.value;

    if (std.mem.eql(u8, event_type, "message_start")) {
        handleMessageStart(root, state);
    } else if (std.mem.eql(u8, event_type, "content_block_start")) {
        handleContentBlockStart(root, state);
    } else if (std.mem.eql(u8, event_type, "content_block_delta")) {
        handleContentBlockDelta(root, state);
    } else if (std.mem.eql(u8, event_type, "content_block_stop")) {
        if (state.in_tool_use) {
            state.emitToolUse();
        }
    } else if (std.mem.eql(u8, event_type, "message_delta")) {
        handleMessageDelta(root, state);
    }
}

fn handleContentBlockStart(root: std.json.Value, state: *StreamState) void {
    const content_block = getObject(root, "content_block") orelse return;
    const block_type = getString(content_block, "type") orelse return;

    if (std.mem.eql(u8, block_type, "tool_use")) {
        const id = getString(content_block, "id") orelse return;
        const name = getString(content_block, "name") orelse return;

        if (state.current_tool_id) |old| state.allocator.free(old);
        if (state.current_tool_name) |old| state.allocator.free(old);

        state.current_tool_id = state.allocator.dupe(u8, id) catch null;
        state.current_tool_name = state.allocator.dupe(u8, name) catch null;
        state.tool_input_buf.clearRetainingCapacity();
        state.in_tool_use = true;
    }
}

fn handleContentBlockDelta(root: std.json.Value, state: *StreamState) void {
    const delta = getObject(root, "delta") orelse return;
    const delta_type = getString(delta, "type") orelse return;

    if (std.mem.eql(u8, delta_type, "text_delta")) {
        const text = getString(delta, "text") orelse return;
        const owned = state.allocator.dupe(u8, text) catch return;
        state.callback(state.ctx, .{ .text_delta = owned });
    } else if (std.mem.eql(u8, delta_type, "input_json_delta")) {
        const partial = getString(delta, "partial_json") orelse return;
        state.tool_input_buf.appendSlice(partial) catch {};
    }
}

fn handleMessageStart(root: std.json.Value, state: *StreamState) void {
    // Extract input_tokens from message_start: { "message": { "usage": { "input_tokens": N } } }
    const message = getObject(root, "message") orelse return;
    const usage = getObject(message, "usage") orelse return;
    const input_tokens = getInteger(usage, "input_tokens") orelse return;
    state.callback(state.ctx, .{ .usage_update = .{ .input_tokens = input_tokens, .output_tokens = 0 } });
}

fn handleMessageDelta(root: std.json.Value, state: *StreamState) void {
    const delta = getObject(root, "delta") orelse return;
    const reason_str = getString(delta, "stop_reason") orelse return;
    const stop_reason = types.StopReason.fromString(reason_str);

    // Extract output_tokens from message_delta: { "usage": { "output_tokens": N } }
    if (getObject(root, "usage")) |usage| {
        const output_tokens = getInteger(usage, "output_tokens") orelse 0;
        state.callback(state.ctx, .{ .usage_update = .{ .input_tokens = 0, .output_tokens = output_tokens } });
    }

    state.callback(state.ctx, .{ .message_end = stop_reason });
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

fn getInteger(val: std.json.Value, key: []const u8) ?u64 {
    switch (val) {
        .object => |obj| {
            if (obj.get(key)) |v| {
                switch (v) {
                    .integer => |i| return if (i >= 0) @intCast(i) else null,
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

/// Write a JSON-escaped string with surrounding quotes.
fn writeJsonString(w: anytype, s: []const u8) !void {
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

test "buildRequestBody produces valid JSON" {
    const allocator = std.testing.allocator;

    const text_block = types.ContentBlock{ .text = "hello" };
    const blocks = [_]types.ContentBlock{text_block};
    const msg = types.Message{ .role = .user, .content = &blocks };
    const messages = [_]types.Message{msg};

    const request = types.ChatRequest{
        .messages = &messages,
        .system_prompt = "You are helpful.",
        .tools = &.{},
        .model = "claude-sonnet-4-20250514",
        .max_tokens = 1024,
        .temperature = null,
    };

    const body = try buildRequestBody(allocator, request);
    defer allocator.free(body);

    // Verify it parses as valid JSON
    const parsed = try std.json.parseFromSlice(std.json.Value, allocator, body, .{});
    defer parsed.deinit();

    const root = parsed.value.object;
    try std.testing.expectEqualStrings("claude-sonnet-4-20250514", root.get("model").?.string);
    try std.testing.expect(root.get("stream").?.bool);
    try std.testing.expectEqual(@as(i64, 1024), root.get("max_tokens").?.integer);
    try std.testing.expectEqualStrings("You are helpful.", root.get("system").?.string);
}

test "buildRequestBody escapes special characters" {
    const allocator = std.testing.allocator;

    const text_block = types.ContentBlock{ .text = "line1\nline2\t\"quoted\"" };
    const blocks = [_]types.ContentBlock{text_block};
    const msg = types.Message{ .role = .user, .content = &blocks };
    const messages = [_]types.Message{msg};

    const request = types.ChatRequest{
        .messages = &messages,
        .system_prompt = "",
        .tools = &.{},
        .model = "claude-sonnet-4-20250514",
        .max_tokens = 1024,
        .temperature = null,
    };

    const body = try buildRequestBody(allocator, request);
    defer allocator.free(body);

    // Must parse successfully (escaping is correct)
    const parsed = try std.json.parseFromSlice(std.json.Value, allocator, body, .{});
    defer parsed.deinit();
}

test "writeJsonString escapes correctly" {
    const allocator = std.testing.allocator;
    var buf = std.ArrayList(u8).init(allocator);
    defer buf.deinit();

    try writeJsonString(buf.writer(), "hello \"world\"\nnewline");
    try std.testing.expectEqualStrings("\"hello \\\"world\\\"\\nnewline\"", buf.items);
}
