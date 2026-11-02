// Builds the installer with local paths remapped in the Rust binary, so panic locations of
// dependencies show /cargo/... instead of C:\Users\<name>\.cargo\... (no user name in the exe).
import { spawnSync } from "node:child_process";
import os from "node:os";
import path from "node:path";

const home = os.homedir();
const remaps = [
  [home, "/home"],
  [process.env.CARGO_HOME || path.join(home, ".cargo"), "/cargo"],
  [process.env.RUSTUP_HOME || path.join(home, ".rustup"), "/rustup"],
  [path.resolve("src-tauri"), "/src"],
];
// rustc: when several prefixes match, the last one wins, so the broad home remap comes first.
const flags = remaps.map(([from, to]) => `--remap-path-prefix=${from}=${to}`);
const existing = process.env.CARGO_ENCODED_RUSTFLAGS ? process.env.CARGO_ENCODED_RUSTFLAGS.split("\x1f") : [];
const env = { ...process.env, CARGO_ENCODED_RUSTFLAGS: [...existing, ...flags].join("\x1f") };
delete env.RUSTFLAGS;

const result = spawnSync("tauri", ["build", ...process.argv.slice(2)], { stdio: "inherit", env, shell: true });
process.exit(result.status ?? 1);
