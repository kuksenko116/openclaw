const std = @import("std");
const Allocator = std.mem.Allocator;
const types = @import("../types.zig");
const agent_mod = @import("../agent.zig");
const streaming = @import("streaming.zig");

/// Ollama native API provider.
///
/// Streams responses from `POST /api/chat` using NDJSON (one JSON object
/// per line), NOT SSE. Tool calls appear in intermediate chunks
/// (done:false) and must be accumulated across ALL chunks before being
/// emitted when the final done:true chunk arrives.
pub const OllamaProvider = struct {
    allocator: Allocator,
    model: []const u8,
    base_url: []const u8,

    const default_base_url = "http://127.0.0.1:11434";

    pub fn create(
        allocator: Allocator,
        model: []const u8,
        base_url: ?[]const u8,
    ) !*OllamaProvider {
        const self = try allocator.create(OllamaProvider);
        self.* = .{
            .allocator = allocator,
            .model = model,
            .base_url = base_url orelse default_base_url,
        };
        return self;
    }

    pub fn deinit(self: *OllamaProvider) void {
        self.allocator.destroy(self);
    }

    /// Stream a chat completion. Calls `callback(ctx, event)` for each AgentEvent.
    pub fn streamChat(
        self: *OllamaProvider,
        allocator: Allocator,
        request: types.ChatRequest,
        ctx: *anyopaque,
        callback: agent_mod.Provider.Callback,
    ) !void {
        const body = try buildRequestBody(allocator, request);
        defer allocator.free(body);

        const url = try resolveUrl(allocator, self.base_url);
        defer allocator.free(url);

        const uri = try std.Uri.parse(url);

        var client = std.http.Client{ .allocator = allocator };
        defer client.deinit();

        var header_buf: [8192]u8 = undefined;
        var req = try client.open(.POST, uri, .{
            .server_header_buffer = &header_buf,
            .extra_headers = &.{
                .{ .name = "content-type", .value = "application/json" },
            },
        });
        defer req.deinit();

        req.transfer_encoding = .{ .content_length = body.len };
        try req.send();
        try req.writeAll(body);
        try req.finish();
        try req.wait();

        if (req.response.status != .ok) {
            return error.HttpError;
        }

        try parseNdjsonStream(allocator, &req, ctx, callback);
    }

    /// Provider vtable glue.
    pub fn streamChatVtable(
        ptr: *anyopaque,
        allocator: Allocator,
        request: types.ChatRequest,
        ctx: *anyopaque,
        callback: agent_mod.Provider.Callback,
    ) anyerror!void {
        const self: *OllamaProvider = @ptrCast(@alignCast(ptr));
        return self.streamChat(allocator, request, ctx, callback);
    }
};

// -- URL resolution --

/// Resolve the Ollama chat endpoint URL.
/// Strips trailing slashes and `/v1` suffix, then appends `/api/chat`.
fn resolveUrl(allocator: Allocator, base_url: []const u8) ![]const u8 {
    var trimmed = std.mem.trimRight(u8, base_url, "/");
    if (std.mem.endsWith(u8, trimmed, "/v1")) {
        trimmed = trimmed[0 .. trimmed.len - 3];
    }
    if (trimmed.len == 0) {
        trimmed = OllamaProvider.default_base_url;
    }
    return std.fmt.allocPrint(allocator, "{s}/api/chat", .{trimmed});
}

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
    try writeStr(w, "stream");
    try w.writeAll(":true");

    // Messages
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
        try writeOllamaMessage(w, msg);
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

    // Options: set num_ctx to avoid Ollama's 4096 default
    try w.writeAll(",");
    try writeStr(w, "options");
    try w.writeAll(":{");
    try writeStr(w, "num_ctx");
    try w.writeAll(":65536");

    if (request.temperature) |temp| {
        try w.writeAll(",");
        try writeStr(w, "temperature");
        try w.writeAll(":");
        try w.print("{d:.2}", .{temp});
    }

    try w.writeAll(",");
    try writeStr(w, "num_predict");
    try w.writeAll(":");
    try w.print("{d}", .{request.max_tokens});
    try w.writeAll("}");

    try w.writeAll("}");
    return buf.toOwnedSlice();
}

fn writeOllamaMessage(w: anytype, msg: types.Message) !void {
    switch (msg.role) {
        .user => {
            var block_idx: usize = 0;
            for (msg.content) |block| {
                switch (block) {
                    .tool_result => |tr| {
                        if (block_idx > 0) try w.writeAll(",");
                        try w.writeAll("{");
                        try writeStr(w, "role");
                        try w.writeAll(":");
                        try writeStr(w, "tool");
                        try w.writeAll(",");
                        try writeStr(w, "content");
                        try w.writeAll(":");
                        try writeStr(w, tr.content);
                        try w.writeAll("}");
                        block_idx += 1;
                    },
                    .text => |t| {
                        if (block_idx > 0) try w.writeAll(",");
                        try w.writeAll("{");
                        try writeStr(w, "role");
                        try w.writeAll(":");
                        try writeStr(w, "user");
                        try w.writeAll(",");
                        try writeStr(w, "content");
                        try w.writeAll(":");
                        try writeStr(w, t);
                        try w.writeAll("}");
                        block_idx += 1;
                    },
                    else => {},
                }
            }
        },
        .assistant => {
            try w.writeAll("{");
            try writeStr(w, "role");
            try w.writeAll(":");
            try writeStr(w, "assistant");
            try w.writeAll(",");
            try writeStr(w, "content");
            try w.writeAll(":");
            try writeStr(w, msg.textContent());

            if (msg.hasToolCalls()) {
                try w.writeAll(",");
                try writeStr(w, "tool_calls");
                try w.writeAll(":[");
                var tc_idx: usize = 0;
                for (msg.content) |block| {
                    switch (block) {
                        .tool_use => |tu| {
                            if (tc_idx > 0) try w.writeAll(",");
                            try w.writeAll("{");
                            try writeStr(w, "function");
                            try w.writeAll(":{");
                            try writeStr(w, "name");
                            try w.writeAll(":");
                            try writeStr(w, tu.name);
                            try w.writeAll(",");
                            try writeStr(w, "arguments");
                            try w.writeAll(":");
                            // arguments is raw JSON
                            try w.writeAll(tu.input);
                            try w.writeAll("}}");
                            tc_idx += 1;
                        },
                        else => {},
                    }
                }
                try w.writeAll("]");
            }

            try w.writeAll("}");
        },
        .system => {
            try w.writeAll("{");
            try writeStr(w, "role");
            try w.writeAll(":");
            try writeStr(w, "system");
            try w.writeAll(",");
            try writeStr(w, "content");
            try w.writeAll(":");
            try writeStr(w, msg.textContent());
            try w.writeAll("}");
        },
    }
}

// -- NDJSON stream parsing --

/// Accumulated tool call from Ollama's intermediate chunks.
const OllamaToolCall = struct {
    name: []const u8,
    arguments_json: []const u8,
};

fn parseNdjsonStream(
    allocator: Allocator,
    req: anytype,
    ctx: *anyopaque,
    callback: agent_mod.Provider.Callback,
) !void {
    streaming.setReadTimeout(req, 600); // Ollama can be slow

    var line_buf = std.ArrayList(u8).init(allocator);
    defer line_buf.deinit();

    var accumulated_tool_calls = std.ArrayList(OllamaToolCall).init(allocator);
    defer {
        for (accumulated_tool_calls.items) |tc| {
            allocator.free(tc.name);
            allocator.free(tc.arguments_json);
        }
        accumulated_tool_calls.deinit();
    }

    var read_buf: [4096]u8 = undefined;
    while (true) {
        const n = req.reader().read(&read_buf) catch |err| {
            if (err == error.WouldBlock) {
                const stderr = std.io.getStdErr().writer();
                stderr.writeAll("\x1b[33mWarning: stream timed out after 600s\x1b[0m\n") catch {};
            }
            break;
        };
        if (n == 0) break;

        try line_buf.appendSlice(read_buf[0..n]);

        // Process complete lines
        while (std.mem.indexOf(u8, line_buf.items, "\n")) |newline_pos| {
            const line = std.mem.trim(u8, line_buf.items[0..newline_pos], " \r\t");

            if (line.len > 0) {
                processNdjsonLine(allocator, line, &accumulated_tool_calls, ctx, callback);
            }

            const consumed = newline_pos + 1;
            const remaining_len = line_buf.items.len - consumed;
            if (remaining_len > 0) {
                std.mem.copyForwards(u8, line_buf.items[0..remaining_len], line_buf.items[consumed..]);
            }
            line_buf.shrinkRetainingCapacity(remaining_len);
        }
    }

    // Handle any remaining data without trailing newline
    const remaining = std.mem.trim(u8, line_buf.items, " \r\t\n");
    if (remaining.len > 0) {
        processNdjsonLine(allocator, remaining, &accumulated_tool_calls, ctx, callback);
    }
}

fn processNdjsonLine(
    allocator: Allocator,
    line: []const u8,
    accumulated_tool_calls: *std.ArrayList(OllamaToolCall),
    ctx: *anyopaque,
    callback: agent_mod.Provider.Callback,
) void {
    const parsed = std.json.parseFromSlice(std.json.Value, allocator, line, .{}) catch return;
    defer parsed.deinit();
    const root = parsed.value;

    // Extract message.content for text deltas
    if (getObject(root, "message")) |message| {
        if (getString(message, "content")) |content| {
            if (content.len > 0) {
                const owned = allocator.dupe(u8, content) catch return;
                callback(ctx, .{ .text_delta = owned });
            }
        }

        // CRITICAL: tool_calls appear in intermediate (done:false) chunks.
        // Must accumulate across ALL chunks.
        if (getArray(message, "tool_calls")) |tool_calls| {
            for (tool_calls) |tc| {
                if (getObject(tc, "function")) |func| {
                    const name = getString(func, "name") orelse continue;
                    const name_owned = allocator.dupe(u8, name) catch continue;

                    const args_json = stringifyValue(allocator, func, "arguments") catch continue;

                    accumulated_tool_calls.append(.{
                        .name = name_owned,
                        .arguments_json = args_json,
                    }) catch {};
                }
            }
        }
    }

    // Check if this is the final chunk
    if (getBool(root, "done")) |done| {
        if (done) {
            // Emit all accumulated tool calls
            for (accumulated_tool_calls.items, 0..) |tc, idx| {
                var id_buf: [64]u8 = undefined;
                const id = std.fmt.bufPrint(&id_buf, "ollama_{d}_{d}", .{ std.time.milliTimestamp(), idx }) catch "ollama_0";
                const id_owned = allocator.dupe(u8, id) catch continue;
                const name_copy = allocator.dupe(u8, tc.name) catch continue;
                const args_copy = allocator.dupe(u8, tc.arguments_json) catch continue;

                callback(ctx, .{ .tool_use = .{
                    .id = id_owned,
                    .name = name_copy,
                    .input = args_copy,
                } });
            }

            // Extract token usage: prompt_eval_count (input) and eval_count (output)
            const input_tokens: u64 = @intCast(@max(getInteger(root, "prompt_eval_count") orelse 0, 0));
            const output_tokens: u64 = @intCast(@max(getInteger(root, "eval_count") orelse 0, 0));
            if (input_tokens > 0 or output_tokens > 0) {
                callback(ctx, .{ .usage_update = .{ .input_tokens = input_tokens, .output_tokens = output_tokens } });
            }

            const stop_reason: types.StopReason = if (accumulated_tool_calls.items.len > 0)
                .tool_use
            else
                .end_turn;

            callback(ctx, .{ .message_end = stop_reason });
        }
    }
}

/// Stringify a JSON value at a given key.
/// If it is a string, return it. If it is an object/array, serialize to JSON.
fn stringifyValue(allocator: Allocator, parent: std.json.Value, key: []const u8) ![]const u8 {
    switch (parent) {
        .object => |obj| {
            const val = obj.get(key) orelse return try allocator.dupe(u8, "{}");
            switch (val) {
                .string => |s| return try allocator.dupe(u8, s),
                .object, .array => {
                    var out_buf = std.ArrayList(u8).init(allocator);
                    errdefer out_buf.deinit();
                    try std.json.stringify(val, .{}, out_buf.writer());
                    return out_buf.toOwnedSlice();
                },
                else => return try allocator.dupe(u8, "{}"),
            }
        },
        else => return try allocator.dupe(u8, "{}"),
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

fn getBool(val: std.json.Value, key: []const u8) ?bool {
    switch (val) {
        .object => |obj| {
            if (obj.get(key)) |v| {
                switch (v) {
                    .bool => |b| return b,
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

// -- Tests --

test "resolveUrl strips trailing slash and v1" {
    const allocator = std.testing.allocator;

    const url1 = try resolveUrl(allocator, "http://localhost:11434/");
    defer allocator.free(url1);
    try std.testing.expectEqualStrings("http://localhost:11434/api/chat", url1);

    const url2 = try resolveUrl(allocator, "http://localhost:11434/v1");
    defer allocator.free(url2);
    try std.testing.expectEqualStrings("http://localhost:11434/api/chat", url2);

    const url3 = try resolveUrl(allocator, "http://localhost:11434");
    defer allocator.free(url3);
    try std.testing.expectEqualStrings("http://localhost:11434/api/chat", url3);
}

test "buildRequestBody includes num_ctx option" {
    const allocator = std.testing.allocator;

    const text_block = types.ContentBlock{ .text = "hello" };
    const blocks = [_]types.ContentBlock{text_block};
    const msg = types.Message{ .role = .user, .content = &blocks };
    const messages = [_]types.Message{msg};

    const request = types.ChatRequest{
        .messages = &messages,
        .system_prompt = "",
        .tools = &.{},
        .model = "llama3.1",
        .max_tokens = 4096,
        .temperature = null,
    };

    const body = try buildRequestBody(allocator, request);
    defer allocator.free(body);

    const parsed = try std.json.parseFromSlice(std.json.Value, allocator, body, .{});
    defer parsed.deinit();

    const root = parsed.value.object;
    const options = root.get("options").?.object;
    try std.testing.expectEqual(@as(i64, 65536), options.get("num_ctx").?.integer);
}
