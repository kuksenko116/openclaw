const std = @import("std");

pub const Config = struct {
    provider: []const u8 = "anthropic",
    model: []const u8 = "claude-sonnet-4-20250514",
    api_key: ?[]const u8 = null,
    base_url: ?[]const u8 = null,
    sessions_dir: ?[]const u8 = null,
    system_prompt: ?[]const u8 = null,
    max_tokens: u32 = 8192,
    temperature: ?f32 = null,
    tools_profile: []const u8 = "full",
    exec_security: []const u8 = "allowlist",
    exec_allowlist: []const []const u8 = &.{},

    /// Tracks which string fields were heap-allocated (vs. compile-time defaults).
    owned: OwnedFields = .{},

    const OwnedFields = packed struct {
        provider: bool = false,
        model: bool = false,
        api_key: bool = false,
        base_url: bool = false,
        sessions_dir: bool = false,
        system_prompt: bool = false,
        tools_profile: bool = false,
        exec_security: bool = false,
    };

    pub fn deinit(self: *Config, allocator: std.mem.Allocator) void {
        if (self.owned.provider) allocator.free(self.provider);
        if (self.owned.model) allocator.free(self.model);
        if (self.owned.api_key) if (self.api_key) |k| allocator.free(k);
        if (self.owned.base_url) if (self.base_url) |u| allocator.free(u);
        if (self.owned.sessions_dir) if (self.sessions_dir) |d| allocator.free(d);
        if (self.owned.system_prompt) if (self.system_prompt) |s| allocator.free(s);
        if (self.owned.tools_profile) allocator.free(self.tools_profile);
        if (self.owned.exec_security) allocator.free(self.exec_security);
        self.* = .{};
    }
};

/// Load config from a JSON file. Falls back to defaults if the file does not exist.
pub fn load(allocator: std.mem.Allocator, path: []const u8) !Config {
    const file_content = std.fs.cwd().readFileAlloc(allocator, path, 1024 * 1024) catch |err| {
        if (err == error.FileNotFound) return Config{};
        return err;
    };
    defer allocator.free(file_content);

    return parseConfigJson(allocator, file_content);
}

fn parseConfigJson(allocator: std.mem.Allocator, content: []const u8) !Config {
    const parsed = std.json.parseFromSlice(std.json.Value, allocator, content, .{}) catch {
        return error.InvalidConfig;
    };
    defer parsed.deinit();

    const root = parsed.value;
    if (root != .object) return error.InvalidConfig;

    var config = Config{};
    const obj = root.object;

    if (obj.get("provider")) |v| {
        if (v == .string) {
            config.provider = try allocator.dupe(u8, v.string);
            config.owned.provider = true;
        }
    }
    if (obj.get("model")) |v| {
        if (v == .string) {
            config.model = try allocator.dupe(u8, v.string);
            config.owned.model = true;
        }
    }
    if (obj.get("api_key")) |v| {
        if (v == .string) {
            config.api_key = try allocator.dupe(u8, v.string);
            config.owned.api_key = true;
        }
    }
    if (obj.get("base_url")) |v| {
        if (v == .string) {
            config.base_url = try allocator.dupe(u8, v.string);
            config.owned.base_url = true;
        }
    }
    if (obj.get("sessions_dir")) |v| {
        if (v == .string) {
            config.sessions_dir = try allocator.dupe(u8, v.string);
            config.owned.sessions_dir = true;
        }
    }
    if (obj.get("system_prompt")) |v| {
        if (v == .string) {
            config.system_prompt = try allocator.dupe(u8, v.string);
            config.owned.system_prompt = true;
        }
    }
    if (obj.get("max_tokens")) |v| {
        if (v == .integer) config.max_tokens = @intCast(v.integer);
    }
    if (obj.get("temperature")) |v| {
        switch (v) {
            .float => config.temperature = @floatCast(v.float),
            .integer => config.temperature = @floatFromInt(v.integer),
            else => {},
        }
    }
    if (obj.get("tools_profile")) |v| {
        if (v == .string) {
            config.tools_profile = try allocator.dupe(u8, v.string);
            config.owned.tools_profile = true;
        }
    }
    if (obj.get("exec_security")) |v| {
        if (v == .string) {
            config.exec_security = try allocator.dupe(u8, v.string);
            config.owned.exec_security = true;
        }
    }

    return config;
}

/// Return the default config file path: ~/.openclaw-cli/config.json
pub fn defaultConfigPath(allocator: std.mem.Allocator) ![]const u8 {
    const home = std.process.getEnvVarOwned(allocator, "HOME") catch |err| {
        if (err == error.EnvironmentVariableNotFound) return error.NoHomeDir;
        return err;
    };
    defer allocator.free(home);

    return std.fs.path.join(allocator, &.{ home, ".openclaw-cli", "config.json" });
}

/// Return the default sessions directory: ~/.openclaw-cli/sessions/
pub fn defaultSessionsDir(allocator: std.mem.Allocator) ![]const u8 {
    const home = std.process.getEnvVarOwned(allocator, "HOME") catch |err| {
        if (err == error.EnvironmentVariableNotFound) return error.NoHomeDir;
        return err;
    };
    defer allocator.free(home);

    return std.fs.path.join(allocator, &.{ home, ".openclaw-cli", "sessions" });
}

/// Resolve ${ENV_VAR} patterns in a string.
/// Returns a newly allocated string with variables expanded.
pub fn resolveApiKey(allocator: std.mem.Allocator, key_str: []const u8) ![]const u8 {
    // Check for ${...} pattern
    if (key_str.len < 4) return allocator.dupe(u8, key_str);

    if (std.mem.startsWith(u8, key_str, "${") and std.mem.endsWith(u8, key_str, "}")) {
        const var_name = key_str[2 .. key_str.len - 1];
        const val = std.process.getEnvVarOwned(allocator, var_name) catch |err| {
            if (err == error.EnvironmentVariableNotFound) {
                const stderr = std.io.getStdErr().writer();
                stderr.print("warning: environment variable '{s}' not set\n", .{var_name}) catch {};
                return allocator.dupe(u8, "");
            }
            return err;
        };
        return val;
    }

    return allocator.dupe(u8, key_str);
}

/// Validate configuration values.
pub fn validate(config: *const Config) !void {
    if (config.provider.len == 0) return error.InvalidConfig;
    if (config.model.len == 0) return error.InvalidConfig;
    if (config.temperature) |t| {
        if (t < 0.0 or t > 2.0) return error.InvalidConfig;
    }
    if (config.max_tokens == 0) return error.InvalidConfig;
}

pub const ConfigError = error{
    InvalidConfig,
    NoHomeDir,
};

// -- Tests --

test "defaultConfigPath returns expected path" {
    const allocator = std.testing.allocator;
    const path = try defaultConfigPath(allocator);
    defer allocator.free(path);

    try std.testing.expect(std.mem.endsWith(u8, path, ".openclaw-cli/config.json"));
}

test "defaultSessionsDir returns expected path" {
    const allocator = std.testing.allocator;
    const path = try defaultSessionsDir(allocator);
    defer allocator.free(path);

    try std.testing.expect(std.mem.endsWith(u8, path, ".openclaw-cli/sessions"));
}

test "resolveApiKey passes through plain strings" {
    const allocator = std.testing.allocator;
    const result = try resolveApiKey(allocator, "sk-12345");
    defer allocator.free(result);

    try std.testing.expectEqualStrings("sk-12345", result);
}

test "resolveApiKey resolves env vars" {
    const allocator = std.testing.allocator;
    // HOME should always be set in test environment
    const result = try resolveApiKey(allocator, "${HOME}");
    defer allocator.free(result);

    try std.testing.expect(result.len > 0);
}

test "parseConfigJson handles valid JSON" {
    const allocator = std.testing.allocator;
    const json =
        \\{"provider":"openai","model":"gpt-4","max_tokens":4096}
    ;
    var cfg = try parseConfigJson(allocator, json);
    defer cfg.deinit(allocator);

    try std.testing.expectEqualStrings("openai", cfg.provider);
    try std.testing.expectEqualStrings("gpt-4", cfg.model);
    try std.testing.expectEqual(@as(u32, 4096), cfg.max_tokens);
}

test "parseConfigJson returns defaults for empty object" {
    const allocator = std.testing.allocator;
    var cfg = try parseConfigJson(allocator, "{}");
    defer cfg.deinit(allocator);

    try std.testing.expectEqualStrings("anthropic", cfg.provider);
    try std.testing.expectEqual(@as(u32, 8192), cfg.max_tokens);
}

test "load returns defaults for missing file" {
    const allocator = std.testing.allocator;
    var cfg = try load(allocator, "/tmp/nonexistent-openclaw-test-config.json");
    defer cfg.deinit(allocator);

    try std.testing.expectEqualStrings("anthropic", cfg.provider);
}
