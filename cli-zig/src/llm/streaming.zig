const std = @import("std");
const Allocator = std.mem.Allocator;

/// Set SO_RCVTIMEO on the underlying socket of an HTTP request so reads
/// won't block forever on a stalled connection. Timeout in seconds.
pub fn setReadTimeout(req: anytype, timeout_secs: u31) void {
    const conn = req.connection orelse return;
    const fd = conn.stream.handle;
    const timeout = std.posix.timeval{ .sec = @intCast(timeout_secs), .usec = 0 };
    std.posix.setsockopt(fd, std.posix.SOL.SOCKET, std.posix.SO.RCVTIMEO, std.mem.asBytes(&timeout)) catch {};
}

/// A single Server-Sent Event parsed from a stream.
pub const SseEvent = struct {
    event: ?[]const u8,
    data: []const u8,
};

/// Incremental SSE parser.
///
/// Feed it chunks of bytes with `feed`. It accumulates lines and emits
/// complete events. Follows the SSE spec:
///   - `event:` sets the event type
///   - `data:` appends to the data buffer (multiple data lines joined with \n)
///   - blank line dispatches the accumulated event
///   - `:` prefix is a comment (ignored)
pub const SseParser = struct {
    allocator: Allocator,
    line_buf: std.ArrayList(u8),
    event_type: ?[]const u8,
    data_parts: std.ArrayList([]const u8),

    pub fn init(allocator: Allocator) SseParser {
        return .{
            .allocator = allocator,
            .line_buf = std.ArrayList(u8).init(allocator),
            .event_type = null,
            .data_parts = std.ArrayList([]const u8).init(allocator),
        };
    }

    pub fn deinit(self: *SseParser) void {
        self.clearParts();
        self.data_parts.deinit();
        self.line_buf.deinit();
        if (self.event_type) |et| self.allocator.free(et);
    }

    /// Feed a chunk of raw bytes. Returns parsed events from this chunk.
    /// Caller owns the returned slice and all strings within it.
    pub fn feed(self: *SseParser, chunk: []const u8) ![]SseEvent {
        var events = std.ArrayList(SseEvent).init(self.allocator);
        errdefer events.deinit();

        try self.line_buf.appendSlice(chunk);

        while (true) {
            const newline_pos = std.mem.indexOf(u8, self.line_buf.items, "\n") orelse break;

            const line = std.mem.trimRight(u8, self.line_buf.items[0..newline_pos], "\r");

            if (line.len == 0) {
                // Blank line: dispatch event if we have data
                if (self.data_parts.items.len > 0) {
                    const joined = try std.mem.join(self.allocator, "\n", self.data_parts.items);
                    const ev_type = self.event_type;
                    try events.append(.{
                        .event = ev_type,
                        .data = joined,
                    });
                    // Free individual data parts (joined is a new allocation)
                    self.clearParts();
                    self.event_type = null;
                }
            } else if (std.mem.startsWith(u8, line, ":")) {
                // Comment line, skip
            } else if (std.mem.startsWith(u8, line, "data:")) {
                const value = std.mem.trimLeft(u8, line["data:".len..], " ");
                const owned = try self.allocator.dupe(u8, value);
                try self.data_parts.append(owned);
            } else if (std.mem.startsWith(u8, line, "event:")) {
                const value = std.mem.trimLeft(u8, line["event:".len..], " ");
                if (self.event_type) |old| self.allocator.free(old);
                self.event_type = try self.allocator.dupe(u8, value);
            }
            // else: unknown field, ignore per SSE spec

            // Remove processed line (including the newline char) from buffer
            const consumed = newline_pos + 1;
            const remaining_len = self.line_buf.items.len - consumed;
            if (remaining_len > 0) {
                std.mem.copyForwards(u8, self.line_buf.items[0..remaining_len], self.line_buf.items[consumed..]);
            }
            self.line_buf.shrinkRetainingCapacity(remaining_len);
        }

        return events.toOwnedSlice();
    }

    fn clearParts(self: *SseParser) void {
        for (self.data_parts.items) |part| {
            self.allocator.free(part);
        }
        self.data_parts.clearRetainingCapacity();
    }
};

/// Parse SSE from a reader. Calls `callback` for each complete event.
/// This is a convenience wrapper around SseParser for readers that
/// support `readAtLeast`.
pub fn parseSse(
    allocator: Allocator,
    reader: anytype,
    callback: *const fn (SseEvent) anyerror!void,
) !void {
    var parser = SseParser.init(allocator);
    defer parser.deinit();

    var buf: [4096]u8 = undefined;
    while (true) {
        const n = reader.read(&buf) catch |err| {
            if (err == error.EndOfStream) break;
            return err;
        };
        if (n == 0) break;

        const events = try parser.feed(buf[0..n]);
        defer allocator.free(events);
        for (events) |ev| {
            try callback(ev);
        }
    }
}

// -- Tests --

test "SseParser parses basic text event" {
    const allocator = std.testing.allocator;
    var parser = SseParser.init(allocator);
    defer parser.deinit();

    const events = try parser.feed("data: hello world\n\n");
    defer {
        for (events) |ev| {
            allocator.free(ev.data);
            if (ev.event) |e| allocator.free(e);
        }
        allocator.free(events);
    }

    try std.testing.expectEqual(@as(usize, 1), events.len);
    try std.testing.expectEqualStrings("hello world", events[0].data);
    try std.testing.expect(events[0].event == null);
}

test "SseParser parses event with type" {
    const allocator = std.testing.allocator;
    var parser = SseParser.init(allocator);
    defer parser.deinit();

    const events = try parser.feed("event: content_block_delta\ndata: {\"type\":\"text\"}\n\n");
    defer {
        for (events) |ev| {
            allocator.free(ev.data);
            if (ev.event) |e| allocator.free(e);
        }
        allocator.free(events);
    }

    try std.testing.expectEqual(@as(usize, 1), events.len);
    try std.testing.expectEqualStrings("content_block_delta", events[0].event.?);
    try std.testing.expectEqualStrings("{\"type\":\"text\"}", events[0].data);
}

test "SseParser handles chunked delivery" {
    const allocator = std.testing.allocator;
    var parser = SseParser.init(allocator);
    defer parser.deinit();

    // First chunk: partial line
    const e1 = try parser.feed("data: hel");
    defer allocator.free(e1);
    try std.testing.expectEqual(@as(usize, 0), e1.len);

    // Second chunk: completes line and event
    const e2 = try parser.feed("lo\n\n");
    defer {
        for (e2) |ev| {
            allocator.free(ev.data);
            if (ev.event) |e| allocator.free(e);
        }
        allocator.free(e2);
    }
    try std.testing.expectEqual(@as(usize, 1), e2.len);
    try std.testing.expectEqualStrings("hello", e2[0].data);
}

test "SseParser joins multi-line data" {
    const allocator = std.testing.allocator;
    var parser = SseParser.init(allocator);
    defer parser.deinit();

    const events = try parser.feed("data: line1\ndata: line2\n\n");
    defer {
        for (events) |ev| {
            allocator.free(ev.data);
            if (ev.event) |e| allocator.free(e);
        }
        allocator.free(events);
    }

    try std.testing.expectEqual(@as(usize, 1), events.len);
    try std.testing.expectEqualStrings("line1\nline2", events[0].data);
}

test "SseParser skips comment lines" {
    const allocator = std.testing.allocator;
    var parser = SseParser.init(allocator);
    defer parser.deinit();

    const events = try parser.feed(": this is a comment\ndata: real data\n\n");
    defer {
        for (events) |ev| {
            allocator.free(ev.data);
            if (ev.event) |e| allocator.free(e);
        }
        allocator.free(events);
    }

    try std.testing.expectEqual(@as(usize, 1), events.len);
    try std.testing.expectEqualStrings("real data", events[0].data);
}

test "SseParser handles multiple events in one chunk" {
    const allocator = std.testing.allocator;
    var parser = SseParser.init(allocator);
    defer parser.deinit();

    const events = try parser.feed("data: first\n\ndata: second\n\n");
    defer {
        for (events) |ev| {
            allocator.free(ev.data);
            if (ev.event) |e| allocator.free(e);
        }
        allocator.free(events);
    }

    try std.testing.expectEqual(@as(usize, 2), events.len);
    try std.testing.expectEqualStrings("first", events[0].data);
    try std.testing.expectEqualStrings("second", events[1].data);
}
