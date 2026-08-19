# SignPath Code Signing Setup

This guide covers the one-time setup to enable code signing for Win-CodexBar release artifacts via SignPath.io (SignPath Foundation).

## Prerequisites

- SignPath Foundation program acceptance (confirmed)
- Admin access to the `nesszer/Win-CodexBar` GitHub repo
- Admin access to the SignPath organization

## Step 1: Accept the SignPath organization invitation

Check your email for a SignPath invitation. Accept it and log in at https://app.signpath.io.

## Step 2: Add GitHub repo secrets

In `nesszer/Win-CodexBar` → Settings → Secrets and variables → Actions → New repository secret, add:

| Secret name | Value | Source |
|---|---|---|
| `SIGNPATH_API_TOKEN` | API token with submitter permissions | SignPath → User → API Tokens |
| `SIGNPATH_ORGANIZATION_ID` | Your SignPath organization ID | SignPath → Organization → Settings |
| `SIGNPATH_PROJECT_SLUG` | `Win-CodexBar` (case-sensitive, capital W and B) | SignPath → Project → Settings |
| `SIGNPATH_SIGNING_POLICY_SLUG` | `test-signing` (initial) | SignPath → Project → Signing Policies |

> **Note:** Use `test-signing` initially — the `release-signing` certificate is pending (CSR not yet issued by SignPath). Switch to `release-signing` in Step 7 after the production certificate is available.

## Step 3: Upload the artifact configuration

1. Zip the artifact configuration:
   ```powershell
   Compress-Archive -Path .signpath/artifact-configuration.xml -DestinationPath .signpath/artifact-configuration.zip
   ```
2. In SignPath → Project → Artifact Configurations → Upload
3. Name it `codexbar-installer` (or your preferred slug)
4. Upload the ZIP file containing `artifact-configuration.xml`

The configuration signs both `CodexBar-<version>-Setup.exe` and `CodexBar-<version>-portable.exe` using Authenticode (embedded signatures).

## Step 4: Configure the Trusted Build System

1. In SignPath → Organization → Trusted Build Systems → Add GitHub.com
2. Install the [SignPath GitHub App](https://github.com/apps/signpath) on the `nesszer/Win-CodexBar` repo
3. Link the Trusted Build System to your SignPath project
4. Set the signing policy to require manual approval (the approver is listed in `docs/CODE_SIGNING.md`)

## Step 5: Optional — Source code and build policies

Create `.signpath/policies/<project-slug>/release-signing.yml` to enforce:
- GitHub-hosted runners only (or Blacksmith runner groups)
- No re-runs of signing builds
- Branch rulesets (force-push prevention, PR requirements)

Example policy file structure:
```yaml
github-policies:
  runners:
    allowed_groups:
      - 'blacksmith-4vcpu-windows-2025'
  build:
    disallow_reruns: true
  branch_rulesets:
    - condition:
        rules:
        - block_force_pushes: true
        - require_pull_request:
            min_required_approvals: 1
      allow_bypass_actors: false
```

## Step 6: Test with the self-signed certificate

SignPath provides a test certificate (the `test-signing` policy). To test:

**Option A — Wait for the next real release:** Simplest. On your next release (e.g. `v0.54.0`), the workflow automatically submits artifacts to SignPath. Download the signed installer and verify:

```powershell
Get-AuthenticodeSignature .\CodexBar-0.54.0-Setup.exe
```

**Option B — Test with a throwaway canonical tag:**

1. Temporarily bump all 5 version files to `0.0.1`:
   - `rust/Cargo.toml`
   - `apps/desktop-tauri/src-tauri/Cargo.toml`
   - `apps/desktop-tauri/package.json`
   - `apps/desktop-tauri/src-tauri/tauri.conf.json`
   - `version.env` (MARKETING_VERSION)
2. Commit: `git commit -am "Temp bump for SignPath test"`
3. Tag and push: `git tag v0.0.1 && git push origin main && git push origin v0.0.1`
4. Wait for the workflow to complete
5. Download the signed exe and verify with `Get-AuthenticodeSignature`
6. Clean up:
   ```powershell
   git tag -d v0.0.1
   git push origin :refs/tags/v0.0.1
   gh release delete v0.0.1 --repo nesszer/Win-CodexBar --yes
   git revert HEAD --no-edit
   git push origin main
   ```

Do NOT use non-canonical tags like `v0.0.0-test` — the release preflight rejects them.

## Step 7: Production certificate

After SignPath reviews the setup and issues the production certificate:
1. Update the signing policy in SignPath to use the production certificate
2. Test with another release tag
3. Verify the certificate chain resolves to a trusted root CA

## Workflow behavior without secrets

If `SIGNPATH_API_TOKEN` is not set (or is empty), the SignPath signing step is skipped and the workflow publishes unsigned artifacts with SHA-256 sidecars — identical to the current behavior. This ensures the release pipeline works during onboarding.

## Files

| File | Purpose |
|---|---|
| `.signpath/artifact-configuration.xml` | SignPath artifact config — defines which files to sign and how |
| `.github/workflows/release.yml` | Release workflow with SignPath signing steps between build and publish |
