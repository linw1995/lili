import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { lstat, mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const [binaryArgument, outputArgument, version, platform, signatureKind] = process.argv.slice(2);
if (!binaryArgument || !outputArgument || !version || !platform || !signatureKind) {
  throw new Error(
    "usage: write-forwarder-manifest <binary> <output> <version> <platform> <signature-kind>",
  );
}

const expectedNames = new Map([
  ["arm64-apple-darwin", "lili-hook"],
  ["x86_64-unknown-linux-gnu", "lili-hook"],
  ["x86_64-pc-windows-msvc", "lili-hook.exe"],
]);
if (!expectedNames.has(platform)) throw new Error(`unsupported forwarder platform: ${platform}`);
if (!["platform-standard", "signed"].includes(signatureKind)) {
  throw new Error(`unsupported signature kind: ${signatureKind}`);
}
if (!/^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)$/.test(version)) {
  throw new Error(`invalid release version: ${version}`);
}

const binary = path.resolve(binaryArgument);
const output = path.resolve(outputArgument);
const metadata = await lstat(binary);
if (!metadata.isFile() || metadata.isSymbolicLink()) {
  throw new Error("forwarder must be a regular file");
}
if (path.basename(binary) !== expectedNames.get(platform)) {
  throw new Error(`unexpected forwarder filename for ${platform}`);
}

const versionResult = spawnSync(binary, ["--version"], {
  encoding: "utf8",
  timeout: 5000,
  windowsHide: true,
});
if (versionResult.error) throw versionResult.error;
if (versionResult.status !== 0 || versionResult.signal !== null) {
  throw new Error("forwarder version probe failed");
}
if (versionResult.stderr !== "" || versionResult.stdout !== `lili-hook ${version}\n`) {
  throw new Error("forwarder reported a different release version");
}

const contents = await readFile(binary);
await mkdir(path.dirname(output), { recursive: true });
await writeFile(
  output,
  `${JSON.stringify(
    {
      schemaVersion: 1,
      product: "Lili",
      component: "lili-hook",
      version,
      reportedVersion: version,
      platform,
      fileName: expectedNames.get(platform),
      signatureKind,
      size: contents.length,
      sha256: createHash("sha256").update(contents).digest("hex"),
    },
    null,
    2,
  )}\n`,
);
