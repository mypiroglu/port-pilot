# Contributing to PortPilot

Thanks for your interest in improving PortPilot! Bug reports, ideas, docs and code are all welcome.

## Reporting bugs

Open an [issue](https://github.com/mypiroglu/port-pilot/issues) and include:

- Your OS and version (e.g. macOS 15.1, Windows 11, Ubuntu 24.04)
- PortPilot version
- What you did, what you expected, and what happened
- A screenshot if it's a UI problem

## Development setup

```bash
npm install
npm run tauri dev
```

See the [README](README.md#-build-from-source) for prerequisites.

## Making a change

1. Fork the repository and create a branch: `git checkout -b feat/my-change`
2. Make your change. Keep pull requests focused on one thing.
3. Check that everything builds and the tests pass:
   ```bash
   npm run build
   cd src-tauri && cargo test && cargo clippy
   ```
4. Commit using a clear message, e.g. `feat: group rows by process` or `fix: crash when PID disappears`.
5. Open a pull request describing **what** changed and **why**. Add a screenshot for UI changes.

## Code guidelines

- **Rust:** all OS access lives in `src-tauri/src/ports.rs`. Keep Tauri commands in `lib.rs` thin.
  Add a test in `ports.rs` for new behaviour where possible. Run `cargo fmt`.
- **Frontend:** search and sort logic belongs in `src/filter.ts`; keep `App.tsx` focused on UI.
- **Safety first:** anything that terminates processes must keep the system-process and self-kill guards.
- **No network calls:** PortPilot works fully offline. Please keep it that way.

## Good first issues

Issues labelled `good first issue` are small and well-scoped. Comment on one to claim it.
