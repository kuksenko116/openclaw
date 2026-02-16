const std = @import("std");

pub const Role = enum {
    user,
    assistant,
    system,

    pub fn toString(self: Role) []const u8 {
        return switch (self) {
            .user => "user",
            .assistant => "assistant",
            .system => "system",
        };
    }

    pub fn fromString(s: []const u8) ?Role {
        const map = std.StaticStringMap(Role).initComptime(.{
            .{ "user", .user },
            .{ "assistant", .assistant },
            .{ "system", .system },
        });
        return map.get(s);
    }
};

pub const StopReason = enum {
    end_turn,
    tool_use,
    max_tokens,

    pub fn fromString(s: []const u8) StopReason {
        const map = std.StaticStringMap(StopReason).initComptime(.{
            .{ "end_turn", .end_turn },
            .{ "stop", .end_turn },
            .{ "tool_use", .tool_use },
            .{ "tool_calls", .tool_use },
            .{ "max_tokens", .max_tokens },
            .{ "length", .max_tokens },
        });
        return map.get(s) orelse .end_turn;
    }
};

pub const ContentBlock = union(enum) {
    text: []const u8,
    tool_use: ToolUse,
    tool_result: ToolResultBlock,
};

pub const ToolUse = struct {
    id: []const u8,
    name: []const u8,
    input: []const u8, // raw JSON string
};

pub const ToolResultBlock = struct {
    tool_use_id: []const u8,
    content: []const u8,
    is_error: bool = false,
};

pub const Message = struct {
    role: Role,
    content: []const ContentBlock,

    pub fn textContent(self: *const Message) []const u8 {
        for (self.content) |block| {
            switch (block) {
                .text => |t| return t,
                else => {},
            }
        }
        return "";
    }

    pub fn hasToolCalls(self: *const Message) bool {
        for (self.content) |block| {
            switch (block) {
                .tool_use => return true,
                else => {},
            }
        }
        return false;
    }
};

pub const AgentEvent = union(enum) {
    text_delta: []const u8,
    tool_use: ToolUse,
    message_end: StopReason,
    usage_update: Usage,
};

pub const ToolDefinition = struct {
    name: []const u8,
    description: []const u8,
    input_schema: []const u8, // raw JSON string
};

pub const ToolResult = struct {
    content: []const u8,
    is_error: bool = false,
};

pub const Usage = struct {
    input_tokens: u64 = 0,
    output_tokens: u64 = 0,
};

pub const ChatRequest = struct {
    messages: []const Message,
    system_prompt: []const u8,
    tools: []const ToolDefinition,
    model: []const u8,
    max_tokens: u32,
    temperature: ?f32 = null,
};

// -- Tests --

test "Role.fromString parses known roles" {
    try std.testing.expectEqual(Role.user, Role.fromString("user").?);
    try std.testing.expectEqual(Role.assistant, Role.fromString("assistant").?);
    try std.testing.expectEqual(Role.system, Role.fromString("system").?);
    try std.testing.expect(Role.fromString("unknown") == null);
}

test "StopReason.fromString normalizes provider variants" {
    try std.testing.expectEqual(StopReason.end_turn, StopReason.fromString("end_turn"));
    try std.testing.expectEqual(StopReason.end_turn, StopReason.fromString("stop"));
    try std.testing.expectEqual(StopReason.tool_use, StopReason.fromString("tool_use"));
    try std.testing.expectEqual(StopReason.tool_use, StopReason.fromString("tool_calls"));
    try std.testing.expectEqual(StopReason.max_tokens, StopReason.fromString("max_tokens"));
    try std.testing.expectEqual(StopReason.max_tokens, StopReason.fromString("length"));
    // Unknown defaults to end_turn
    try std.testing.expectEqual(StopReason.end_turn, StopReason.fromString("garbage"));
}

test "Message.textContent returns first text block" {
    const blocks = [_]ContentBlock{
        .{ .text = "hello world" },
    };
    const msg = Message{ .role = .user, .content = &blocks };
    try std.testing.expectEqualStrings("hello world", msg.textContent());
}

test "Message.textContent returns empty for non-text" {
    const blocks = [_]ContentBlock{
        .{ .tool_use = .{ .id = "t1", .name = "bash", .input = "{}" } },
    };
    const msg = Message{ .role = .assistant, .content = &blocks };
    try std.testing.expectEqualStrings("", msg.textContent());
}

test "Message.hasToolCalls detects tool_use blocks" {
    const with_tool = [_]ContentBlock{
        .{ .text = "running command" },
        .{ .tool_use = .{ .id = "t1", .name = "bash", .input = "{}" } },
    };
    const msg_with = Message{ .role = .assistant, .content = &with_tool };
    try std.testing.expect(msg_with.hasToolCalls());

    const without_tool = [_]ContentBlock{
        .{ .text = "just text" },
    };
    const msg_without = Message{ .role = .assistant, .content = &without_tool };
    try std.testing.expect(!msg_without.hasToolCalls());
}
