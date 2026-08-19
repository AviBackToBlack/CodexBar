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
| `SIGNPATH_PROJECT_SLUG` | Project slug (e.g. `win-codexbar`) | SignPath → Project → Settings |
| `SIGNPATH_SIGNING_POLICY_SLUG` | Signing policy slug (e.g. `release-signing`) | SignPath → Project → Signing Policies |

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

SignPath provides a test certificate during onboarding. To test:

1. Ensure all secrets from Step 2 are set
2. Push a test tag (e.g. `v0.0.0-test`)
3. The Release workflow will:
   - Build artifacts
   - Upload unsigned artifacts as GitHub Actions artifacts
   - Submit to SignPath for signing
   - Replace unsigned files with signed versions
   - Publish a draft GitHub release
4. Verify the signed artifacts:
   ```powershell
   Get-AuthenticodeSignature .\CodexBar-0.0.0-test-Setup.exe
   ```
   The signature should show "SignPath Foundation" as the publisher.

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
