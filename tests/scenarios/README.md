# Oikome acceptance environment

One command starts one complete acceptance workspace. It uses real Oikome nodes, real files and real API calls, not a mock `NodeClient`, and never reads or changes `~/Oikome`.

```bash
npm run acceptance
```

This opens the main workspace at `http://127.0.0.1:7342` and prints a second peer's loopback address. The main workspace contains:

- the four ordinary first-run examples;
- local in-place media, text, code, tables, damaged and unsafe files;
- a real CSV connector writing a local transactional collection;
- collections on a mounted-NAS-folder test double and a drive-sync-folder test double;
- an encrypted collection on a real RustFS server started inside the generated directory;
- a real second node that accepted the media collection through the ordinary contact/share flow.

The mounted folder and sync folder are named as test doubles, not as WebDAV, Google Drive or Dropbox. No real cloud account is configured implicitly and no fake connected state is shown. WebDAV and provider API coverage remains in their dedicated tests and real-account checks.

For a completely fresh first-run experience:

```bash
npm run acceptance -- --first-run
```

This opens Welcome without pre-seeding identity, examples, storage or grants. Configuration and example creation happen through the actual UI. The full and first-run modes use different generated state, and each start recreates only `tests/.generated/acceptance` after checking its marker. Repeating the command while the same mode is running opens the existing nodes instead of resetting them.

Use `--open-peer` to open the second node too, `--port 0` for a random main port, or `--no-open` to skip opening browser windows. Stop with Ctrl+C before changing modes. Node snapshots, identities, generated media, service data and login links stay in the ignored generated directory.

Run the isolated tests with:

```bash
npm run test:acceptance
```

They cover the complete workspace, real first-run setup, clean restarts and refusal to replace an unmarked directory. Tests use temporary directories and remove them afterwards. Build the app and node before running acceptance after changing their code: `npm run build` and `cargo node`.
