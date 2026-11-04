<!-- Title: one Conventional Commits header, e.g. feat(api): add contacts -->

## Summary

<!-- What changes for users or for the architecture, and why it is needed. -->

## Related issue

<!-- Link the issue or roadmap phase, or write N/A for a small, self-contained change. -->

## Documentation impact

<!-- Select exactly one option. -->

- [ ] No documentation update is required.
- [ ] `docs/architecture.md` (and `docs/prd.md` for app changes) and/or `AGENT.md` are updated in this PR.
- [ ] A follow-up is required (explain below).

## Validation

<!-- List the exact commands you ran and their results. Say which checks were
     left to CI (full workspace tests, the browser-node specs, the Docker
     nodes) and why. -->

## Checklist

<!-- Check each applicable item, or explain why it does not apply. -->

- [ ] The change is focused and includes tests, or a reason tests are unnecessary.
- [ ] Behavior and compatibility changes are documented.
- [ ] New dependencies and copied code have permissive licenses, are recorded
      in `NOTICE`, and pass `cargo deny check licenses`.
- [ ] No credentials, private endpoints, personal paths, generated data, or
      build artifacts are included.
- [ ] Security implications for identity (DID, UCAN), grants, the network, and
      untrusted input have been considered.
- [ ] Rust changes passed fmt, clippy (`-D warnings`) and the tests of the
      touched crates; TypeScript changes passed `npm run typecheck`.
