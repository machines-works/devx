# devx

## Build & Test

```bash
cargo test --all        # run all tests
cargo fmt --check       # check formatting
cargo clippy -- -D warnings  # lint
```

All three must pass before pushing to main — CI enforces this.

## Release

Release workflow triggers on `v*` tags. To release:

```bash
git tag v0.0.2-alpha
git push origin v0.0.2-alpha
```

This builds binaries for darwin-arm64, linux-x86_64, linux-arm64 and creates a GitHub release with artifacts.

## Deploy Landing Page

The landing page (`site/`) is hosted on Vercel under `ale-space/site` at `devx.machines.works`. There is no auto-deploy from GitHub — deploy manually:

```bash
vercel deploy --prod site/
```

The Vercel GitHub app is not installed on the `machines-works` org, so pushes do not auto-deploy.
