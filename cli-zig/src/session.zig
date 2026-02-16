const std = @import("std");
const types = @import("types.zig");
const Message = types.Message;
const ContentBlock = types.ContentBlock;
const Role = types.Role;

pub const Session = struct {
    id: []const u8,
    messages: std.ArrayList(Message),
    path: []const u8,
    allocator: std.mem.Allocator,
    dirty: bool = false,

    /// Create a new empty session.
    pub fn init(allocator: std.mem.Allocator, id: []const u8, path: []const u8) Session {
        return .{
            .id = id,
            .messages = std.ArrayList(Message).init(allocator),
            .path = path,
            .allocator = allocator,
        };
    }

    /// Free all owned memory: content block slices and their heap-allocated strings.
    pub fn deinit(self: *Session) void {
        for (self.messages.items) |msg| {
            freeMessage(self.allocator, msg);
        }
        self.messages.deinit();
    }

    /// Load a session from a JSONL file. Returns an empty session if the file
    /// does not exist.
    pub fn load(allocator: std.mem.Allocator, path: []const u8) !Session {
        const id = sessionIdFromPath(path);
        var session = Session.init(allocator, id, path);

        const content = std.fs.cwd().readFileAlloc(allocator, path, 50 * 1024 * 1024) catch |err| {
            if (err == error.FileNotFound) return session;
            return err;
        };
        defer allocator.free(content);

        var iter = std.mem.splitScalar(u8, content, '\n');
        while (iter.next()) |line| {
            const trimmed = std.mem.trim(u8, line, " \r\t");
            if (trimmed.len == 0) continue;

            const msg = parseMessageLine(allocator, trimmed) catch continue;
            try session.messages.append(msg);
        }

        return session;
    }

    /// Append a user text message. The text is duped — caller retains ownership of the original.
    pub fn addUserMessage(self: *Session, text: []const u8) !void {
        const content_slice = try self.allocator.alloc(ContentBlock, 1);
        content_slice[0] = .{ .text = try self.allocator.dupe(u8, text) };

        try self.messages.append(.{
            .role = .user,
            .content = content_slice,
        });
        self.dirty = true;
    }

    /// Append an assistant message with the given content blocks.
    pub fn addAssistantMessage(self: *Session, content: []const ContentBlock) !void {
        try self.messages.append(.{
            .role = .assistant,
            .content = content,
        });
        self.dirty = true;
    }

    /// Persist the session to disk using atomic write (write .tmp, rename).
    pub fn save(self: *Session) !void {
        if (!self.dirty) return;

        // Ensure the parent directory exists
        const dir_path = std.fs.path.dirname(self.path) orelse ".";
        std.fs.makeDirAbsolute(dir_path) catch |err| {
            if (err != error.PathAlreadyExists) return err;
        };

        var tmp_buf: [std.fs.max_path_bytes]u8 = undefined;
        const timestamp = std.time.milliTimestamp();
        const tmp_path = std.fmt.bufPrint(&tmp_buf, "{s}.tmp.{d}", .{ self.path, timestamp }) catch
            return error.NameTooLong;

        // Write to temp file
        const tmp_file = try std.fs.createFileAbsolute(tmp_path, .{});
        errdefer {
            tmp_file.close();
            std.fs.deleteFileAbsolute(tmp_path) catch {};
        }

        var buffered = std.io.bufferedWriter(tmp_file.writer());
        const writer = buffered.writer();

        for (self.messages.items) |msg| {
            try writeMessageLine(writer, msg);
            try writer.writeByte('\n');
        }
        try buffered.flush();
        tmp_file.close();

        // Atomic rename
        std.fs.renameAbsolute(tmp_path, self.path) catch |err| {
            std.fs.deleteFileAbsolute(tmp_path) catch {};
            return err;
        };

        self.dirty = false;
    }

    /// Return the message slice for building a ChatRequest.
    pub fn getMessages(self: *const Session) []const Message {
        return self.messages.items;
    }
};

/// Sanitize a session name to prevent path traversal.
///
/// Takes the basename (strips directory components), strips leading dots
/// and trailing `.jsonl` extension. Returns error if the result is empty.
pub fn sanitizeSessionName(allocator: std.mem.Allocator, name: []const u8) ![]const u8 {
    // Take the final path component only (strips traversal like "../../etc/")
    const basename = std.fs.path.basename(name);

    // Strip leading dots (prevents ".." and hidden files)
    var start: usize = 0;
    while (start < basename.len and basename[start] == '.') : (start += 1) {}
    var cleaned = basename[start..];

    // Strip .jsonl or .json extension if present
    if (std.mem.endsWith(u8, cleaned, ".jsonl")) {
        cleaned = cleaned[0 .. cleaned.len - 6];
    } else if (std.mem.endsWith(u8, cleaned, ".json")) {
        cleaned = cleaned[0 .. cleaned.len - 5];
    }

    if (cleaned.len == 0) return error.InvalidSessionName;

    return try allocator.dupe(u8, cleaned);
}

/// Build a sanitized session file path.
pub fn sessionPath(allocator: std.mem.Allocator, sessions_dir: []const u8, name: []const u8) ![]const u8 {
    const safe_name = try sanitizeSessionName(allocator, name);
    defer allocator.free(safe_name);

    const filename = try std.fmt.allocPrint(allocator, "{s}.jsonl", .{safe_name});
    defer allocator.free(filename);

    return try std.fs.path.join(allocator, &.{ sessions_dir, filename });
}

/// Extract a session id from its file path (basename without extension).
fn sessionIdFromPath(path: []const u8) []const u8 {
    const basename = std.fs.path.basename(path);
    if (std.mem.lastIndexOfScalar(u8, basename, '.')) |dot| {
        return basename[0..dot];
    }
    return basename;
}

/// Parse a single JSONL line into a Message.
/// All strings are duped from the JSON parse tree so it can be freed immediately.
fn parseMessageLine(allocator: std.mem.Allocator, line: []const u8) !Message {
    const parsed = try std.json.parseFromSlice(std.json.Value, allocator, line, .{});
    defer parsed.deinit();

    const root = parsed.value;
    if (root != .object) return error.InvalidSession;

    const obj = root.object;

    // Determine role
    const role_str = if (obj.get("role")) |v| (if (v == .string) v.string else null) else null;
    const role = if (role_str) |s| (Role.fromString(s) orelse return error.InvalidSession) else return error.InvalidSession;

    // Parse content -- can be a string or array of content blocks
    const content_val = obj.get("content") orelse return error.InvalidSession;

    switch (content_val) {
        .string => |s| {
            const blocks = try allocator.alloc(ContentBlock, 1);
            blocks[0] = .{ .text = try allocator.dupe(u8, s) };
            return .{ .role = role, .content = blocks };
        },
        .array => |arr| {
            var blocks = try allocator.alloc(ContentBlock, arr.items.len);
            for (arr.items, 0..) |item, i| {
                blocks[i] = try parseContentBlock(allocator, item);
            }
            return .{ .role = role, .content = blocks };
        },
        else => return error.InvalidSession,
    }
}

fn parseContentBlock(allocator: std.mem.Allocator, val: std.json.Value) !ContentBlock {
    if (val != .object) return error.InvalidSession;
    const obj = val.object;

    const type_str = if (obj.get("type")) |v| (if (v == .string) v.string else null) else null;
    const block_type = type_str orelse return error.InvalidSession;

    if (std.mem.eql(u8, block_type, "text")) {
        const text = if (obj.get("text")) |v| (if (v == .string) v.string else null) else null;
        return .{ .text = try allocator.dupe(u8, text orelse "") };
    }

    if (std.mem.eql(u8, block_type, "tool_use")) {
        return .{ .tool_use = .{
            .id = try dupeOrDefault(allocator, obj, "id", ""),
            .name = try dupeOrDefault(allocator, obj, "name", ""),
            .input = try dupeOrDefault(allocator, obj, "input", "{}"),
        } };
    }

    if (std.mem.eql(u8, block_type, "tool_result")) {
        return .{ .tool_result = .{
            .tool_use_id = try dupeOrDefault(allocator, obj, "tool_use_id", ""),
            .content = try dupeOrDefault(allocator, obj, "content", ""),
            .is_error = if (obj.get("is_error")) |v| (if (v == .bool) v.bool else false) else false,
        } };
    }

    return error.InvalidSession;
}

/// Extract a string field from a JSON object, dupe it, or return a duped default.
fn dupeOrDefault(allocator: std.mem.Allocator, obj: std.json.ObjectMap, key: []const u8, default: []const u8) ![]const u8 {
    if (obj.get(key)) |v| {
        if (v == .string) return try allocator.dupe(u8, v.string);
    }
    return try allocator.dupe(u8, default);
}

/// Write a Message as a single JSON line.
fn writeMessageLine(writer: anytype, msg: Message) !void {
    try writer.writeAll("{\"role\":\"");
    try writer.writeAll(msg.role.toString());
    try writer.writeAll("\",\"content\":[");

    for (msg.content, 0..) |block, i| {
        if (i > 0) try writer.writeByte(',');
        try writeContentBlock(writer, block);
    }

    try writer.writeAll("]}");
}

fn writeContentBlock(writer: anytype, block: ContentBlock) !void {
    switch (block) {
        .text => |text| {
            try writer.writeAll("{\"type\":\"text\",\"text\":");
            try writeJsonString(writer, text);
            try writer.writeByte('}');
        },
        .tool_use => |tu| {
            try writer.writeAll("{\"type\":\"tool_use\",\"id\":");
            try writeJsonString(writer, tu.id);
            try writer.writeAll(",\"name\":");
            try writeJsonString(writer, tu.name);
            try writer.writeAll(",\"input\":");
            try writeJsonString(writer, tu.input);
            try writer.writeByte('}');
        },
        .tool_result => |tr| {
            try writer.writeAll("{\"type\":\"tool_result\",\"tool_use_id\":");
            try writeJsonString(writer, tr.tool_use_id);
            try writer.writeAll(",\"content\":");
            try writeJsonString(writer, tr.content);
            try writer.writeAll(",\"is_error\":");
            if (tr.is_error) {
                try writer.writeAll("true");
            } else {
                try writer.writeAll("false");
            }
            try writer.writeByte('}');
        },
    }
}

/// Free a message and all its owned content.
fn freeMessage(allocator: std.mem.Allocator, msg: Message) void {
    for (msg.content) |block| {
        freeContentBlock(allocator, block);
    }
    allocator.free(msg.content);
}

/// Free heap-allocated strings inside a content block.
fn freeContentBlock(allocator: std.mem.Allocator, block: ContentBlock) void {
    switch (block) {
        .text => |t| allocator.free(t),
        .tool_use => |tu| {
            allocator.free(tu.id);
            allocator.free(tu.name);
            allocator.free(tu.input);
        },
        .tool_result => |tr| {
            allocator.free(tr.tool_use_id);
            allocator.free(tr.content);
        },
    }
}

/// Write a JSON-escaped string (with surrounding quotes).
fn writeJsonString(writer: anytype, s: []const u8) !void {
    try writer.writeByte('"');
    for (s) |c| {
        switch (c) {
            '"' => try writer.writeAll("\\\""),
            '\\' => try writer.writeAll("\\\\"),
            '\n' => try writer.writeAll("\\n"),
            '\r' => try writer.writeAll("\\r"),
            '\t' => try writer.writeAll("\\t"),
            else => {
                if (c < 0x20) {
                    try writer.print("\\u{x:0>4}", .{c});
                } else {
                    try writer.writeByte(c);
                }
            },
        }
    }
    try writer.writeByte('"');
}

// -- Tests --

test "sessionIdFromPath extracts id" {
    try std.testing.expectEqualStrings("my-session", sessionIdFromPath("/home/user/.openclaw-cli/sessions/my-session.jsonl"));
    try std.testing.expectEqualStrings("abc", sessionIdFromPath("abc.jsonl"));
    try std.testing.expectEqualStrings("noext", sessionIdFromPath("noext"));
}

test "writeJsonString escapes special characters" {
    var buf: [256]u8 = undefined;
    var fbs = std.io.fixedBufferStream(&buf);
    const writer = fbs.writer();

    try writeJsonString(writer, "hello \"world\"\nnewline");
    const result = fbs.getWritten();
    try std.testing.expectEqualStrings("\"hello \\\"world\\\"\\nnewline\"", result);
}

test "Session round-trip through addUserMessage" {
    const allocator = std.testing.allocator;
    var session = Session.init(allocator, "test", "/tmp/test.jsonl");
    defer session.deinit();

    try session.addUserMessage("hello");
    try std.testing.expectEqual(@as(usize, 1), session.messages.items.len);
    try std.testing.expectEqual(Role.user, session.messages.items[0].role);
    try std.testing.expectEqualStrings("hello", session.messages.items[0].textContent());
    // session.deinit() frees all content blocks and strings
}

test "sanitizeSessionName normal name" {
    const allocator = std.testing.allocator;
    const result = try sanitizeSessionName(allocator, "my-session");
    defer allocator.free(result);
    try std.testing.expectEqualStrings("my-session", result);
}

test "sanitizeSessionName strips path traversal" {
    const allocator = std.testing.allocator;
    const result = try sanitizeSessionName(allocator, "../../etc/passwd");
    defer allocator.free(result);
    try std.testing.expectEqualStrings("passwd", result);
}

test "sanitizeSessionName strips leading dots" {
    const allocator = std.testing.allocator;
    const result = try sanitizeSessionName(allocator, ".hidden");
    defer allocator.free(result);
    try std.testing.expectEqualStrings("hidden", result);
}

test "sanitizeSessionName strips jsonl extension" {
    const allocator = std.testing.allocator;
    const result = try sanitizeSessionName(allocator, "test.jsonl");
    defer allocator.free(result);
    try std.testing.expectEqualStrings("test", result);
}

test "sanitizeSessionName rejects empty" {
    const allocator = std.testing.allocator;
    const result = sanitizeSessionName(allocator, "");
    try std.testing.expectError(error.InvalidSessionName, result);
}

test "sanitizeSessionName rejects dots only" {
    const allocator = std.testing.allocator;
    const result = sanitizeSessionName(allocator, "..");
    try std.testing.expectError(error.InvalidSessionName, result);
}

test "sessionPath stays within directory" {
    const allocator = std.testing.allocator;
    const path = try sessionPath(allocator, "/home/user/sessions", "../../etc/passwd");
    defer allocator.free(path);
    try std.testing.expectEqualStrings("/home/user/sessions/passwd.jsonl", path);
}
