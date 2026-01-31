import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

import { describe, expect, it } from "vitest";

import type { OpenClawPluginApi, OpenClawPluginToolContext } from "../../../src/plugins/types.js";
import { createLobsterTool } from "./lobster-tool.js";

async function writeFakeLobsterScript(scriptBody: string, prefix = "openclaw-lobster-plugin-") {
  const dir = await fs.mkdtemp(path.join(os.tmpdir(), prefix));
  const isWindows = process.platform === "win32";

  if (isWindows) {
    const scriptPath = path.join(dir, "lobster.js");
    const cmdPath = path.join(dir, "lobster.cmd");
    await fs.writeFile(scriptPath, scriptBody, { encoding: "utf8" });
    const cmd = `@echo off\r\n"${process.execPath}" "${scriptPath}" %*\r\n`;
    await fs.writeFile(cmdPath, cmd, { encoding: "utf8" });
    return { dir, binPath: cmdPath };
  }

  const binPath = path.join(dir, "lobster");
  const file = `#!/usr/bin/env node\n${scriptBody}\n`;
  await fs.writeFile(binPath, file, { encoding: "utf8", mode: 0o755 });
  return { dir, binPath };
}

async function writeFakeLobster(params: { payload: unknown }) {
  const scriptBody =
    `const payload = ${JSON.stringify(params.payload)};\n` +
    `process.stdout.write(JSON.stringify(payload));\n`;
  return await writeFakeLobsterScript(scriptBody);
}

function fakeApi(overrides: Partial<OpenClawPluginApi> = {}): OpenClawPluginApi {
  return {
    id: "lobster",
    name: "lobster",
    source: "test",
    config: {} as any,
    pluginConfig: {},
    runtime: { version: "test" } as any,
    logger: { info() {}, warn() {}, error() {}, debug() {} },
    registerTool() {},
    registerHttpHandler() {},
    registerChannel() {},
    registerGatewayMethod() {},
    registerCli() {},
    registerService() {},
    registerProvider() {},
    registerHook() {},
    registerHttpRoute() {},
    registerCommand() {},
    on() {},
    resolvePath: (p) => p,
    ...overrides,
  };
}

function fakeCtx(overrides: Partial<OpenClawPluginToolContext> = {}): OpenClawPluginToolContext {
  return {
    config: {} as any,
    workspaceDir: "/tmp",
    agentDir: "/tmp",
    agentId: "main",
    sessionKey: "main",
    messageChannel: undefined,
    agentAccountId: undefined,
    sandboxed: false,
    ...overrides,
  };
}

describe("lobster plugin tool", () => {
  it("runs lobster and returns parsed envelope in details", async () => {
    const fake = await writeFakeLobster({
      payload: { ok: true, status: "ok", output: [{ hello: "world" }], requiresApproval: null },
    });

    const originalPath = process.env.PATH;
    process.env.PATH = `${fake.dir}${path.delimiter}${originalPath ?? ""}`;

    try {
      const tool = createLobsterTool(fakeApi());
      const res = await tool.execute("call1", {
        action: "run",
        pipeline: "noop",
        timeoutMs: 1000,
      });

      expect(res.details).toMatchObject({ ok: true, status: "ok" });
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("tolerates noisy stdout before the JSON envelope", async () => {
    const payload = { ok: true, status: "ok", output: [], requiresApproval: null };
    const { dir } = await writeFakeLobsterScript(
      `const payload = ${JSON.stringify(payload)};\n` +
        `console.log("noise before json");\n` +
        `process.stdout.write(JSON.stringify(payload));\n`,
      "openclaw-lobster-plugin-noisy-",
    );

    const originalPath = process.env.PATH;
    process.env.PATH = `${dir}${path.delimiter}${originalPath ?? ""}`;

    try {
      const tool = createLobsterTool(fakeApi());
      const res = await tool.execute("call-noisy", {
        action: "run",
        pipeline: "noop",
        timeoutMs: 1000,
      });

      expect(res.details).toMatchObject({ ok: true, status: "ok" });
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("requires absolute lobsterPath when provided (even though it is ignored)", async () => {
    const fake = await writeFakeLobster({
      payload: { ok: true, status: "ok", output: [{ hello: "world" }], requiresApproval: null },
    });

    const originalPath = process.env.PATH;
    process.env.PATH = `${fake.dir}${path.delimiter}${originalPath ?? ""}`;

    try {
      const tool = createLobsterTool(fakeApi());
      await expect(
        tool.execute("call2", {
          action: "run",
          pipeline: "noop",
          lobsterPath: "./lobster",
        }),
      ).rejects.toThrow(/absolute path/);
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("rejects lobsterPath (deprecated) when invalid", async () => {
    const fake = await writeFakeLobster({
      payload: { ok: true, status: "ok", output: [{ hello: "world" }], requiresApproval: null },
    });

    const originalPath = process.env.PATH;
    process.env.PATH = `${fake.dir}${path.delimiter}${originalPath ?? ""}`;

    try {
      const tool = createLobsterTool(fakeApi());
      await expect(
        tool.execute("call2b", {
          action: "run",
          pipeline: "noop",
          lobsterPath: "/bin/bash",
        }),
      ).rejects.toThrow(/lobster executable/);
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("rejects absolute cwd", async () => {
    const tool = createLobsterTool(fakeApi());
    await expect(
      tool.execute("call2c", {
        action: "run",
        pipeline: "noop",
        cwd: "/tmp",
      }),
    ).rejects.toThrow(/cwd must be a relative path/);
  });

  it("rejects cwd that escapes the gateway working directory", async () => {
    const tool = createLobsterTool(fakeApi());
    await expect(
      tool.execute("call2d", {
        action: "run",
        pipeline: "noop",
        cwd: "../../etc",
      }),
    ).rejects.toThrow(/must stay within/);
  });

  it("uses pluginConfig.lobsterPath when provided", async () => {
    const fake = await writeFakeLobster({
      payload: { ok: true, status: "ok", output: [{ hello: "world" }], requiresApproval: null },
    });

    // Ensure `lobster` is NOT discoverable via PATH, while still allowing our
    // fake lobster (a Node script with `#!/usr/bin/env node`) to run.
    const originalPath = process.env.PATH;
    process.env.PATH = path.dirname(process.execPath);

    try {
      const tool = createLobsterTool(fakeApi({ pluginConfig: { lobsterPath: fake.binPath } }));
      const res = await tool.execute("call-plugin-config", {
        action: "run",
        pipeline: "noop",
        timeoutMs: 1000,
      });

      expect(res.details).toMatchObject({ ok: true, status: "ok" });
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("rejects invalid JSON from lobster", async () => {
    const { dir } = await writeFakeLobsterScript(
      `process.stdout.write("nope");\n`,
      "openclaw-lobster-plugin-bad-",
    );

    const originalPath = process.env.PATH;
    process.env.PATH = `${dir}${path.delimiter}${originalPath ?? ""}`;

    try {
      const tool = createLobsterTool(fakeApi());
      await expect(
        tool.execute("call3", {
          action: "run",
          pipeline: "noop",
        }),
      ).rejects.toThrow(/invalid JSON/);
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("rejects invalid JSON envelope from lobster", async () => {
    const { dir } = await writeFakeLobsterScript(
      `process.stdout.write(JSON.stringify({ hello: "world" }));\n`,
      "openclaw-lobster-plugin-bad-envelope-",
    );

    const originalPath = process.env.PATH;
    process.env.PATH = `${dir}${path.delimiter}${originalPath ?? ""}`;

    try {
      const tool = createLobsterTool(fakeApi());
      await expect(
        tool.execute("call3b", {
          action: "run",
          pipeline: "noop",
        }),
      ).rejects.toThrow(/invalid JSON envelope/);
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("requires action", async () => {
    const tool = createLobsterTool(fakeApi());
    await expect(tool.execute("call-action-missing", {})).rejects.toThrow(/action required/);
    await expect(tool.execute("call-action-empty", { action: "  " })).rejects.toThrow(
      /action required/,
    );
  });

  it("rejects unknown action", async () => {
    const tool = createLobsterTool(fakeApi());
    await expect(tool.execute("call-action-unknown", { action: "nope" })).rejects.toThrow(
      /Unknown action/,
    );
  });

  it("validates run/resume parameters", async () => {
    const tool = createLobsterTool(fakeApi());

    await expect(tool.execute("call-run-missing-pipeline", { action: "run" })).rejects.toThrow(
      /pipeline required/,
    );
    await expect(
      tool.execute("call-run-empty-pipeline", { action: "run", pipeline: "  " }),
    ).rejects.toThrow(/pipeline required/);

    await expect(tool.execute("call-resume-missing-token", { action: "resume" })).rejects.toThrow(
      /token required/,
    );
    await expect(
      tool.execute("call-resume-empty-token", { action: "resume", token: "  ", approve: true }),
    ).rejects.toThrow(/token required/);

    await expect(
      tool.execute("call-resume-missing-approve", { action: "resume", token: "t" }),
    ).rejects.toThrow(/approve required/);
    await expect(
      tool.execute("call-resume-non-boolean-approve", {
        action: "resume",
        token: "t",
        approve: "yes",
      }),
    ).rejects.toThrow(/approve required/);
  });

  it("rejects pluginConfig.lobsterPath when not absolute", async () => {
    const tool = createLobsterTool(fakeApi({ pluginConfig: { lobsterPath: "./lobster" } }));
    await expect(
      tool.execute("call-plugin-config-relative", {
        action: "run",
        pipeline: "noop",
      }),
    ).rejects.toThrow(/absolute path/);
  });

  it("rejects pluginConfig.lobsterPath when it does not exist", async () => {
    const dir = await fs.mkdtemp(path.join(os.tmpdir(), "openclaw-lobster-plugin-missing-"));
    const missingPath = path.join(dir, "lobster");

    const tool = createLobsterTool(fakeApi({ pluginConfig: { lobsterPath: missingPath } }));
    await expect(
      tool.execute("call-plugin-config-missing", {
        action: "run",
        pipeline: "noop",
      }),
    ).rejects.toThrow(/must exist/);
  });

  it("rejects pluginConfig.lobsterPath when it points to a directory", async () => {
    const dir = await fs.mkdtemp(path.join(os.tmpdir(), "openclaw-lobster-plugin-dir-"));
    const lobsterDir = path.join(dir, "lobster");
    await fs.mkdir(lobsterDir);

    const tool = createLobsterTool(fakeApi({ pluginConfig: { lobsterPath: lobsterDir } }));
    await expect(
      tool.execute("call-plugin-config-dir", {
        action: "run",
        pipeline: "noop",
      }),
    ).rejects.toThrow(/point to a file/);
  });

  it("rejects pluginConfig.lobsterPath when it is not executable (posix)", async () => {
    if (process.platform === "win32") {
      return;
    }

    const dir = await fs.mkdtemp(path.join(os.tmpdir(), "openclaw-lobster-plugin-nonexec-"));
    const binPath = path.join(dir, "lobster");
    await fs.writeFile(binPath, "#!/usr/bin/env node\nprocess.stdout.write('[]')\n", {
      encoding: "utf8",
      mode: 0o644,
    });

    const tool = createLobsterTool(fakeApi({ pluginConfig: { lobsterPath: binPath } }));
    await expect(
      tool.execute("call-plugin-config-nonexec", {
        action: "run",
        pipeline: "noop",
      }),
    ).rejects.toThrow(/executable/);
  });

  it("trims pluginConfig.lobsterPath", async () => {
    const fake = await writeFakeLobster({
      payload: { ok: true, status: "ok", output: [], requiresApproval: null },
    });

    // Ensure `lobster` is NOT discoverable via PATH, while still allowing our
    // fake lobster (a Node script with `#!/usr/bin/env node`) to run.
    const originalPath = process.env.PATH;
    process.env.PATH = path.dirname(process.execPath);

    try {
      const tool = createLobsterTool(
        fakeApi({ pluginConfig: { lobsterPath: `  ${fake.binPath}  ` } }),
      );
      const res = await tool.execute("call-plugin-config-trim", {
        action: "run",
        pipeline: "noop",
        timeoutMs: 1000,
      });

      expect(res.details).toMatchObject({ ok: true, status: "ok" });
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("ignores non-string pluginConfig.lobsterPath", async () => {
    const fake = await writeFakeLobster({
      payload: { ok: true, status: "ok", output: [], requiresApproval: null },
    });

    const originalPath = process.env.PATH;
    process.env.PATH = `${fake.dir}${path.delimiter}${originalPath ?? ""}`;

    try {
      const tool = createLobsterTool(fakeApi({ pluginConfig: { lobsterPath: 123 as any } }));
      const res = await tool.execute("call-plugin-config-non-string", {
        action: "run",
        pipeline: "noop",
        timeoutMs: 1000,
      });

      expect(res.details).toMatchObject({ ok: true, status: "ok" });
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("validates deprecated lobsterPath even though it is ignored", async () => {
    const fake = await writeFakeLobster({
      payload: { ok: true, status: "ok", output: [], requiresApproval: null },
    });

    // Ensure `lobster` is NOT discoverable via PATH, while still allowing our
    // fake lobster to run via plugin config.
    const originalPath = process.env.PATH;
    process.env.PATH = path.dirname(process.execPath);

    try {
      const tool = createLobsterTool(fakeApi({ pluginConfig: { lobsterPath: fake.binPath } }));
      await expect(
        tool.execute("call-deprecated-invalid-with-plugin-config", {
          action: "run",
          pipeline: "noop",
          lobsterPath: "/bin/bash",
          timeoutMs: 1000,
        }),
      ).rejects.toThrow(/lobster executable/);
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("rejects lobsterPath injection attempts", async () => {
    const tool = createLobsterTool(fakeApi());
    await expect(
      tool.execute("call-lobsterpath-injection", {
        action: "run",
        pipeline: "noop",
        lobsterPath: "/tmp/lobster --help",
      }),
    ).rejects.toThrow(/lobster executable/);
  });

  it("defaults cwd when empty or non-string", async () => {
    const payload = {
      ok: true,
      status: "ok",
      output: [{ cwd: "__REPLACED__" }],
      requiresApproval: null,
    };

    const { dir } = await writeFakeLobsterScript(
      `const payload = ${JSON.stringify(payload)};\n` +
        `payload.output[0].cwd = process.cwd();\n` +
        `process.stdout.write(JSON.stringify(payload));\n`,
      "openclaw-lobster-plugin-cwd-default-",
    );

    const originalPath = process.env.PATH;
    process.env.PATH = `${dir}${path.delimiter}${originalPath ?? ""}`;

    try {
      const tool = createLobsterTool(fakeApi());
      const res1 = await tool.execute("call-cwd-empty", {
        action: "run",
        pipeline: "noop",
        cwd: "   ",
        timeoutMs: 1000,
      });
      expect((res1.details as any).output[0].cwd).toBe(process.cwd());

      const res2 = await tool.execute("call-cwd-non-string", {
        action: "run",
        pipeline: "noop",
        cwd: 123,
        timeoutMs: 1000,
      });
      expect((res2.details as any).output[0].cwd).toBe(process.cwd());
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("uses trimmed relative cwd within the gateway working directory", async () => {
    const relDir = `.vitest-lobster-cwd-${Date.now()}`;
    const absDir = path.join(process.cwd(), relDir);
    await fs.mkdir(absDir);

    const payload = {
      ok: true,
      status: "ok",
      output: [{ cwd: "__REPLACED__" }],
      requiresApproval: null,
    };

    const { dir } = await writeFakeLobsterScript(
      `const payload = ${JSON.stringify(payload)};\n` +
        `payload.output[0].cwd = process.cwd();\n` +
        `process.stdout.write(JSON.stringify(payload));\n`,
      "openclaw-lobster-plugin-cwd-allowed-",
    );

    const originalPath = process.env.PATH;
    process.env.PATH = `${dir}${path.delimiter}${originalPath ?? ""}`;

    try {
      const tool = createLobsterTool(fakeApi());
      const res = await tool.execute("call-cwd-trim", {
        action: "run",
        pipeline: "noop",
        cwd: `  ${relDir}  `,
        timeoutMs: 1000,
      });
      expect((res.details as any).output[0].cwd).toBe(absDir);
    } finally {
      process.env.PATH = originalPath;
      await fs.rm(absDir, { recursive: true, force: true });
    }
  });

  it("rejects cwd that escapes via symlink", async () => {
    if (process.platform === "win32") {
      // Windows symlink creation can require elevated privileges in CI.
      return;
    }

    const outside = await fs.mkdtemp(path.join(os.tmpdir(), "openclaw-lobster-plugin-outside-"));
    const linkName = `.vitest-lobster-symlink-${Date.now()}`;
    const linkPath = path.join(process.cwd(), linkName);

    await fs.symlink(outside, linkPath, "dir");

    try {
      const tool = createLobsterTool(fakeApi());
      await expect(
        tool.execute("call-cwd-symlink-escape", {
          action: "run",
          pipeline: "noop",
          cwd: linkName,
        }),
      ).rejects.toThrow(/must stay within/);
    } finally {
      await fs.rm(linkPath, { recursive: true, force: true });
      await fs.rm(outside, { recursive: true, force: true });
    }
  });

  it("enforces maxStdoutBytes", async () => {
    const { dir } = await writeFakeLobsterScript(
      `process.stdout.write("x".repeat(20_000));\n`,
      "openclaw-lobster-plugin-stdout-limit-",
    );

    const originalPath = process.env.PATH;
    process.env.PATH = `${dir}${path.delimiter}${originalPath ?? ""}`;

    try {
      const tool = createLobsterTool(fakeApi());
      await expect(
        tool.execute("call-stdout-limit", {
          action: "run",
          pipeline: "noop",
          timeoutMs: 2000,
          maxStdoutBytes: 1024,
        }),
      ).rejects.toThrow(/maxStdoutBytes/);
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("times out lobster subprocess", async () => {
    const { dir } = await writeFakeLobsterScript(
      `setTimeout(() => {}, 10_000);\n`,
      "openclaw-lobster-plugin-timeout-",
    );

    const originalPath = process.env.PATH;
    process.env.PATH = `${dir}${path.delimiter}${originalPath ?? ""}`;

    try {
      const tool = createLobsterTool(fakeApi());
      await expect(
        tool.execute("call-timeout", {
          action: "run",
          pipeline: "noop",
          timeoutMs: 250,
        }),
      ).rejects.toThrow(/timed out/);
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("removes NODE_OPTIONS containing --inspect from child env", async () => {
    const payload = {
      ok: true,
      status: "ok",
      output: [{ nodeOptions: "__REPLACED__" }],
      requiresApproval: null,
    };

    const { dir } = await writeFakeLobsterScript(
      `const payload = ${JSON.stringify(payload)};\n` +
        `payload.output[0].nodeOptions = process.env.NODE_OPTIONS ?? null;\n` +
        `process.stdout.write(JSON.stringify(payload));\n`,
      "openclaw-lobster-plugin-node-options-",
    );

    const originalPath = process.env.PATH;
    const originalNodeOptions = process.env.NODE_OPTIONS;
    process.env.PATH = `${dir}${path.delimiter}${originalPath ?? ""}`;
    process.env.NODE_OPTIONS = "--inspect=0";

    try {
      const tool = createLobsterTool(fakeApi());
      const res = await tool.execute("call-node-options", {
        action: "run",
        pipeline: "noop",
        timeoutMs: 1000,
      });

      expect((res.details as any).output[0].nodeOptions).toBeNull();
    } finally {
      process.env.PATH = originalPath;
      process.env.NODE_OPTIONS = originalNodeOptions;
    }
  });

  it("runs on Windows when lobster is only available as lobster.cmd on PATH (shell fallback)", async () => {
    if (process.platform !== "win32") {
      return;
    }

    const fake = await writeFakeLobster({
      payload: { ok: true, status: "ok", output: [{ hello: "win" }], requiresApproval: null },
    });

    const originalPath = process.env.PATH;
    process.env.PATH = `${fake.dir}${path.delimiter}${originalPath ?? ""}`;

    try {
      const tool = createLobsterTool(fakeApi());
      const res = await tool.execute("call-win-shell-fallback", {
        action: "run",
        pipeline: "noop",
        timeoutMs: 2000,
      });

      expect(res.details).toMatchObject({ ok: true, status: "ok" });
    } finally {
      process.env.PATH = originalPath;
    }
  });

  it("can be gated off in sandboxed contexts", async () => {
    const api = fakeApi();
    const factoryTool = (ctx: OpenClawPluginToolContext) => {
      if (ctx.sandboxed) {
        return null;
      }
      return createLobsterTool(api);
    };

    expect(factoryTool(fakeCtx({ sandboxed: true }))).toBeNull();
    expect(factoryTool(fakeCtx({ sandboxed: false }))?.name).toBe("lobster");
  });
});
