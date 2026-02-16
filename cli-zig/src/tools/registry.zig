const std = @import("std");
const Allocator = std.mem.Allocator;
const types = @import("../types.zig");
const bash = @import("bash.zig");
const file_ops = @import("file_ops.zig");

/// Tool registry. Dispatches tool calls to the right handler,
/// manages tool definitions, and enforces execution policy.
pub const ToolRegistry = struct {
    allocator: Allocator,
    profile: []const u8,
    exec_security: []const u8,
    exec_allowlist: []const []const u8,

    pub fn init(
        allocator: Allocator,
        profile: []const u8,
        exec_security: []const u8,
        exec_allowlist: []const []const u8,
    ) ToolRegistry {
        return .{
            .allocator = allocator,
            .profile = profile,
            .exec_security = exec_security,
            .exec_allowlist = exec_allowlist,
        };
    }

    /// Vtable glue: execute through opaque pointer.
    pub fn executeVtable(ptr: *anyopaque, name: []const u8, input: []const u8) anyerror!types.ToolResult {
        const self: *ToolRegistry = @ptrCast(@alignCast(ptr));
        return self.execute(name, input);
    }

    /// Vtable glue: definitions through opaque pointer.
    pub fn definitionsVtable(ptr: *anyopaque) []const types.ToolDefinition {
        const self: *ToolRegistry = @ptrCast(@alignCast(ptr));
        return self.definitions();
    }

    /// Execute a tool by name with JSON arguments.
    pub fn execute(self: *ToolRegistry, name: []const u8, args_json: []const u8) !types.ToolResult {
        if (!self.isAllowed(name)) {
            return .{ .content = "Tool not allowed by policy", .is_error = true };
        }

        if (std.mem.eql(u8, name, "bash")) {
            return self.executeBash(args_json);
        } else if (std.mem.eql(u8, name, "read")) {
            return self.executeRead(args_json);
        } else if (std.mem.eql(u8, name, "write")) {
            return self.executeWrite(args_json);
        } else if (std.mem.eql(u8, name, "edit")) {
            return self.executeEdit(args_json);
        } else if (std.mem.eql(u8, name, "glob")) {
            return self.executeGlob(args_json);
        } else if (std.mem.eql(u8, name, "grep")) {
            return self.executeGrep(args_json);
        }

        const msg = try std.fmt.allocPrint(self.allocator, "Unknown tool: {s}", .{name});
        return .{ .content = msg, .is_error = true };
    }

    /// Return all tool definitions for this registry.
    pub fn definitions(self: *ToolRegistry) []const types.ToolDefinition {
        _ = self;
        return &tool_defs;
    }

    /// Check if a tool is allowed by the current policy.
    pub fn isAllowed(self: *ToolRegistry, name: []const u8) bool {
        // "full" profile allows everything
        if (std.mem.eql(u8, self.profile, "full")) return true;

        // "minimal" profile: only read and glob
        if (std.mem.eql(u8, self.profile, "minimal")) {
            return std.mem.eql(u8, name, "read") or std.mem.eql(u8, name, "glob");
        }

        // "coding" profile: all file ops + bash
        if (std.mem.eql(u8, self.profile, "coding")) {
            return std.mem.eql(u8, name, "bash") or
                std.mem.eql(u8, name, "read") or
                std.mem.eql(u8, name, "write") or
                std.mem.eql(u8, name, "edit") or
                std.mem.eql(u8, name, "glob") or
                std.mem.eql(u8, name, "grep");
        }

        // Default: allow all standard tools
        return true;
    }

    // -- Tool dispatch helpers --

    fn executeBash(self: *ToolRegistry, args_json: []const u8) !types.ToolResult {
        const args = parseArgs(BashArgs, self.allocator, args_json) catch {
            return .{ .content = "Invalid arguments for bash tool", .is_error = true };
        };
        defer args.deinit();

        // Check exec security policy
        if (!self.isExecAllowed(args.value.command)) {
            return .{ .content = "Command not allowed by exec security policy", .is_error = true };
        }

        const timeout: u64 = if (args.value.timeout_ms) |t| t else 30_000;
        return bash.execute(self.allocator, args.value.command, timeout);
    }

    fn executeRead(self: *ToolRegistry, args_json: []const u8) !types.ToolResult {
        const args = parseArgs(ReadArgs, self.allocator, args_json) catch {
            return .{ .content = "Invalid arguments for read tool", .is_error = true };
        };
        defer args.deinit();
        if (isSensitivePath(args.value.file_path))
            return .{ .content = "Access denied: path targets a sensitive location", .is_error = true };
        return file_ops.readFile(self.allocator, args.value.file_path, args.value.offset, args.value.limit);
    }

    fn executeWrite(self: *ToolRegistry, args_json: []const u8) !types.ToolResult {
        const args = parseArgs(WriteArgs, self.allocator, args_json) catch {
            return .{ .content = "Invalid arguments for write tool", .is_error = true };
        };
        defer args.deinit();
        if (isSensitivePath(args.value.file_path))
            return .{ .content = "Access denied: path targets a sensitive location", .is_error = true };
        return file_ops.writeFile(self.allocator, args.value.file_path, args.value.content);
    }

    fn executeEdit(self: *ToolRegistry, args_json: []const u8) !types.ToolResult {
        const args = parseArgs(EditArgs, self.allocator, args_json) catch {
            return .{ .content = "Invalid arguments for edit tool", .is_error = true };
        };
        defer args.deinit();
        if (isSensitivePath(args.value.file_path))
            return .{ .content = "Access denied: path targets a sensitive location", .is_error = true };
        return file_ops.editFile(self.allocator, args.value.file_path, args.value.old_string, args.value.new_string);
    }

    fn executeGlob(self: *ToolRegistry, args_json: []const u8) !types.ToolResult {
        const args = parseArgs(GlobArgs, self.allocator, args_json) catch {
            return .{ .content = "Invalid arguments for glob tool", .is_error = true };
        };
        defer args.deinit();
        return file_ops.globFiles(self.allocator, args.value.pattern, args.value.path);
    }

    fn executeGrep(self: *ToolRegistry, args_json: []const u8) !types.ToolResult {
        const args = parseArgs(GrepArgs, self.allocator, args_json) catch {
            return .{ .content = "Invalid arguments for grep tool", .is_error = true };
        };
        defer args.deinit();
        return file_ops.grepFiles(self.allocator, args.value.pattern, args.value.path, args.value.include);
    }

    /// Check whether a command is allowed by the exec security policy.
    ///
    /// In allowlist mode, extracts the first token (command name) and matches
    /// it against patterns. Commands with shell chaining operators (;, &&, ||,
    /// backticks, $()) are rejected to prevent injection.
    fn isExecAllowed(self: *ToolRegistry, command: []const u8) bool {
        if (std.mem.eql(u8, self.exec_security, "full")) return true;
        if (std.mem.eql(u8, self.exec_security, "deny")) return false;

        // Reject commands with shell chaining/injection metacharacters
        if (std.mem.indexOf(u8, command, ";") != null) return false;
        if (std.mem.indexOf(u8, command, "&&") != null) return false;
        if (std.mem.indexOf(u8, command, "|") != null) return false; // catches | and ||
        if (std.mem.indexOf(u8, command, "`") != null) return false;
        if (std.mem.indexOf(u8, command, "$(") != null) return false;
        if (std.mem.indexOf(u8, command, "\n") != null) return false;
        if (std.mem.indexOf(u8, command, "\r") != null) return false;
        if (std.mem.indexOf(u8, command, "<<") != null) return false;
        if (std.mem.indexOf(u8, command, "<(") != null) return false;
        if (std.mem.indexOf(u8, command, ">(") != null) return false;

        // Extract the first token (command name)
        const trimmed = std.mem.trim(u8, command, " \t\r\n");
        var iter = std.mem.splitScalar(u8, trimmed, ' ');
        const first_token = iter.first();
        if (first_token.len == 0) return false;

        // Get the binary name (basename) from the first token
        const bin_name = std.fs.path.basename(first_token);

        // Check against allowlist patterns
        for (self.exec_allowlist) |pattern| {
            if (std.mem.eql(u8, bin_name, pattern)) return true;
        }
        return false;
    }
};

// -- Argument types --

const BashArgs = struct {
    command: []const u8,
    timeout_ms: ?u64 = null,
};

const ReadArgs = struct {
    file_path: []const u8,
    offset: ?usize = null,
    limit: ?usize = null,
};

const WriteArgs = struct {
    file_path: []const u8,
    content: []const u8,
};

const EditArgs = struct {
    file_path: []const u8,
    old_string: []const u8,
    new_string: []const u8,
};

const GlobArgs = struct {
    pattern: []const u8,
    path: ?[]const u8 = null,
};

const GrepArgs = struct {
    pattern: []const u8,
    path: ?[]const u8 = null,
    include: ?[]const u8 = null,
};

fn parseArgs(comptime T: type, allocator: Allocator, json: []const u8) !std.json.Parsed(T) {
    return std.json.parseFromSlice(T, allocator, json, .{
        .ignore_unknown_fields = true,
    });
}

// -- Tool definitions --

pub const tool_defs = [_]types.ToolDefinition{
    .{
        .name = "bash",
        .description = "Execute a bash command and return its output.",
        .input_schema =
        \\{
        \\  "type": "object",
        \\  "required": ["command"],
        \\  "properties": {
        \\    "command": {
        \\      "type": "string",
        \\      "description": "The bash command to execute"
        \\    },
        \\    "timeout_ms": {
        \\      "type": "integer",
        \\      "description": "Timeout in milliseconds (default: 30000)"
        \\    }
        \\  }
        \\}
        ,
    },
    .{
        .name = "read",
        .description = "Read a file from the filesystem with line numbers.",
        .input_schema =
        \\{
        \\  "type": "object",
        \\  "required": ["file_path"],
        \\  "properties": {
        \\    "file_path": {
        \\      "type": "string",
        \\      "description": "Absolute path to the file to read"
        \\    },
        \\    "offset": {
        \\      "type": "integer",
        \\      "description": "Line number to start reading from (1-based)"
        \\    },
        \\    "limit": {
        \\      "type": "integer",
        \\      "description": "Maximum number of lines to read (default: 2000)"
        \\    }
        \\  }
        \\}
        ,
    },
    .{
        .name = "write",
        .description = "Write content to a file. Creates parent directories if needed.",
        .input_schema =
        \\{
        \\  "type": "object",
        \\  "required": ["file_path", "content"],
        \\  "properties": {
        \\    "file_path": {
        \\      "type": "string",
        \\      "description": "Absolute path to the file to write"
        \\    },
        \\    "content": {
        \\      "type": "string",
        \\      "description": "Content to write to the file"
        \\    }
        \\  }
        \\}
        ,
    },
    .{
        .name = "edit",
        .description = "Perform exact string replacement in a file. Fails if the string is not found or not unique.",
        .input_schema =
        \\{
        \\  "type": "object",
        \\  "required": ["file_path", "old_string", "new_string"],
        \\  "properties": {
        \\    "file_path": {
        \\      "type": "string",
        \\      "description": "Absolute path to the file to edit"
        \\    },
        \\    "old_string": {
        \\      "type": "string",
        \\      "description": "The exact text to find and replace"
        \\    },
        \\    "new_string": {
        \\      "type": "string",
        \\      "description": "The replacement text"
        \\    }
        \\  }
        \\}
        ,
    },
    .{
        .name = "glob",
        .description = "Find files matching a glob pattern.",
        .input_schema =
        \\{
        \\  "type": "object",
        \\  "required": ["pattern"],
        \\  "properties": {
        \\    "pattern": {
        \\      "type": "string",
        \\      "description": "Glob pattern to match (e.g. '**/*.zig')"
        \\    },
        \\    "path": {
        \\      "type": "string",
        \\      "description": "Base directory to search from"
        \\    }
        \\  }
        \\}
        ,
    },
    .{
        .name = "grep",
        .description = "Search for a pattern in files using grep.",
        .input_schema =
        \\{
        \\  "type": "object",
        \\  "required": ["pattern"],
        \\  "properties": {
        \\    "pattern": {
        \\      "type": "string",
        \\      "description": "The regex pattern to search for"
        \\    },
        \\    "path": {
        \\      "type": "string",
        \\      "description": "File or directory to search in"
        \\    },
        \\    "include": {
        \\      "type": "string",
        \\      "description": "Glob filter for files (e.g. '*.zig')"
        \\    }
        \\  }
        \\}
        ,
    },
};

/// Sensitive path patterns that file tools should not access.
const denied_patterns = [_][]const u8{
    "/etc/shadow",
    "/etc/gshadow",
    "/etc/master.passwd",
    "/.ssh/",
    "/.gnupg/",
    "/.aws/credentials",
    "/.config/gcloud/",
    "/.docker/config.json",
};

/// Check if a file path targets a known sensitive location.
/// Resolves `.` and `..` components before checking.
fn isSensitivePath(path: []const u8) bool {
    // Use realpath-style resolution to normalize away .. and .
    // std.fs.path.resolve handles this lexically (no filesystem access needed)
    var buf: [std.fs.max_path_bytes]u8 = undefined;
    const normalized = std.fmt.bufPrint(&buf, "{s}", .{path}) catch path;

    // Manually collapse /../ and /./ sequences for the check
    var norm_buf: [std.fs.max_path_bytes]u8 = undefined;
    const resolved = resolveDots(normalized, &norm_buf) catch normalized;

    for (denied_patterns) |pattern| {
        if (std.mem.indexOf(u8, resolved, pattern) != null) return true;
    }
    return false;
}

/// Lexically resolve `.` and `..` in a path.
fn resolveDots(path: []const u8, buf: []u8) ![]const u8 {
    var components = std.ArrayList([]const u8).init(std.heap.page_allocator);
    defer components.deinit();

    var iter = std.mem.splitScalar(u8, path, '/');
    while (iter.next()) |comp| {
        if (comp.len == 0 or std.mem.eql(u8, comp, ".")) {
            continue;
        } else if (std.mem.eql(u8, comp, "..")) {
            if (components.items.len > 0) {
                _ = components.pop();
            }
        } else {
            try components.append(comp);
        }
    }

    var pos: usize = 0;
    // Preserve leading / for absolute paths
    if (path.len > 0 and path[0] == '/') {
        buf[0] = '/';
        pos = 1;
    }
    for (components.items, 0..) |comp, i| {
        if (i > 0) {
            buf[pos] = '/';
            pos += 1;
        }
        @memcpy(buf[pos..][0..comp.len], comp);
        pos += comp.len;
    }
    return buf[0..pos];
}

// -- Tests --

test "ToolRegistry.isAllowed respects minimal profile" {
    const allocator = std.testing.allocator;
    var reg = ToolRegistry.init(allocator, "minimal", "full", &.{});
    try std.testing.expect(reg.isAllowed("read"));
    try std.testing.expect(reg.isAllowed("glob"));
    try std.testing.expect(!reg.isAllowed("bash"));
    try std.testing.expect(!reg.isAllowed("write"));
}

test "ToolRegistry.isAllowed respects full profile" {
    const allocator = std.testing.allocator;
    var reg = ToolRegistry.init(allocator, "full", "full", &.{});
    try std.testing.expect(reg.isAllowed("bash"));
    try std.testing.expect(reg.isAllowed("read"));
    try std.testing.expect(reg.isAllowed("write"));
    try std.testing.expect(reg.isAllowed("anything"));
}

test "ToolRegistry.isAllowed respects coding profile" {
    const allocator = std.testing.allocator;
    var reg = ToolRegistry.init(allocator, "coding", "full", &.{});
    try std.testing.expect(reg.isAllowed("bash"));
    try std.testing.expect(reg.isAllowed("read"));
    try std.testing.expect(reg.isAllowed("edit"));
    try std.testing.expect(reg.isAllowed("grep"));
    try std.testing.expect(!reg.isAllowed("web_fetch"));
}

test "ToolRegistry.definitions returns all tools" {
    const allocator = std.testing.allocator;
    var reg = ToolRegistry.init(allocator, "full", "full", &.{});
    const defs = reg.definitions();
    try std.testing.expectEqual(@as(usize, 6), defs.len);
    try std.testing.expectEqualStrings("bash", defs[0].name);
    try std.testing.expectEqualStrings("grep", defs[5].name);
}

test "ToolRegistry.execute returns error for unknown tool" {
    const allocator = std.testing.allocator;
    var reg = ToolRegistry.init(allocator, "full", "full", &.{});
    const result = try reg.execute("nonexistent_tool", "{}");
    defer allocator.free(result.content);
    try std.testing.expect(result.is_error);
    try std.testing.expect(std.mem.indexOf(u8, result.content, "Unknown tool") != null);
}

test "isExecAllowed rejects pipes" {
    const allocator = std.testing.allocator;
    const allowlist = [_][]const u8{"git"};
    var reg = ToolRegistry.init(allocator, "full", "allowlist", &allowlist);
    try std.testing.expect(!reg.isExecAllowed("git log | rm -rf /"));
}

test "isExecAllowed rejects newline injection" {
    const allocator = std.testing.allocator;
    const allowlist = [_][]const u8{"git"};
    var reg = ToolRegistry.init(allocator, "full", "allowlist", &allowlist);
    try std.testing.expect(!reg.isExecAllowed("git status\nrm -rf /"));
    try std.testing.expect(!reg.isExecAllowed("git status\rrm -rf /"));
}

test "isExecAllowed rejects heredocs" {
    const allocator = std.testing.allocator;
    const allowlist = [_][]const u8{"cat"};
    var reg = ToolRegistry.init(allocator, "full", "allowlist", &allowlist);
    try std.testing.expect(!reg.isExecAllowed("cat << EOF"));
}

test "isExecAllowed rejects process substitution" {
    const allocator = std.testing.allocator;
    const allowlist = [_][]const u8{"diff"};
    var reg = ToolRegistry.init(allocator, "full", "allowlist", &allowlist);
    try std.testing.expect(!reg.isExecAllowed("diff <(cat /etc/shadow) /dev/null"));
    try std.testing.expect(!reg.isExecAllowed("diff >(tee /tmp/out) /dev/null"));
}

test "ToolRegistry.execute enforces exec security deny" {
    const allocator = std.testing.allocator;
    var reg = ToolRegistry.init(allocator, "full", "deny", &.{});
    const result = try reg.execute("bash", "{\"command\":\"echo hi\"}");
    try std.testing.expect(result.is_error);
    try std.testing.expect(std.mem.indexOf(u8, result.content, "not allowed") != null);
}
