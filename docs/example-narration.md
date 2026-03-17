# Example narration

This is a realistic example of what your coding agent receives when you narrate.
You never see this directly (unless you [yank to clipboard](commands.md#attend-narrate-yank));
it's delivered behind the scenes as a prompt to the agent.

The narration below could have been produced by speaking for about 30 seconds
while navigating code, selecting text on a web page, and running a command. Five
of the seven context sources appear here (voice, editor snapshots, file diffs,
browser selections, shell commands):

---

I want to refactor this config loading. Right now it reads the whole file and
parses it inline

`src/config.rs:31`:
```rust
pub fn load(path: &Path) -> Result<Config> {
    let raw = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&raw)?;
    Ok(config)
}
```

but I think we should support hierarchical config, walking up parent directories
and merging. Something like the approach described here

[Configuration merging](https://doc.rust-lang.org/cargo/reference/config.html):
> Cargo allows local configuration for a particular package as well as global
> configuration. It looks for configuration files in the current directory and
> all parent directories.

I already started changing the struct to support optional fields

`src/config.rs`:
```diff
-pub struct Config {
-    pub engine: Engine,
-    pub model: PathBuf,
-    pub timeout: u64,
+pub struct Config {
+    pub engine: Option<Engine>,
+    pub model: Option<PathBuf>,
+    pub timeout: Option<Duration>,
 }
```

`src/config.rs:12`:
```rust
pub struct Config {
    pub engine: Option<Engine>,
    pub model: Option<PathBuf>,
    pub timeout: Option<Duration>,
}
```

Let me just make sure the tests still pass

```fish
$ cargo nextest run -p attend config  # exit 0, 1.4s
```

OK good. So the plan is: walk upward from cwd, collect all config files, parse
each one, then merge them with closer files taking precedence for scalar values
and concatenating arrays.

---

Notice how the narration interleaves the developer's spoken thoughts (plain
text) with the code they were looking at (editor snapshots), changes they made
(file diffs), documentation they referenced in the browser (browser selections),
and commands they ran (shell commands). The agent receives all of this as a
single chronological narrative.

External selections (text highlighted in other apps), clipboard content (copied
text or images), and redaction markers (for files outside the project scope) can
also appear; they aren't shown in this particular example.
