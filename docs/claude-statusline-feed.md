# Claude Code statusLine feed

CodexBar's Claude Code statusLine integration is an explicit, off-by-default local observation path. It is not an
Anthropic API, an OAuth feature, or a credential reader. Claude Code sends JSON to the command configured in the
user's `statusLine` setting; CodexBar's bundled CLI keeps only the official rate-limit windows that are available.

## What it can show

The helper accepts only:

- `rate_limits.five_hour.used_percentage` and `rate_limits.five_hour.resets_at`
- `rate_limits.seven_day.used_percentage` and `rate_limits.seven_day.resets_at`

Each window is independent. A numeric `used_percentage` in `0...100` makes that window usable even when the other
window is absent. `resets_at` is optional and is kept only when it is a numeric Unix epoch within CodexBar's
2000-through-2100 sanity range; missing, string-valued, or implausible reset metadata is omitted without losing the
percentage. Unknown fields and unrelated statusLine data are discarded.
CodexBar never persists raw stdin, account identity, cwd, repository, prompt/session text, cost, model, or unknown
fields. The observation contains one or both allowlisted windows, capture time, schema version, and a one-way profile
identifier. It expires after 15 minutes; timestamps more than five minutes in the future are rejected.

The feed is deliberately anonymous. While global **Disable Keychain access** is enabled, a fresh observation may
stand alone as the ambient Claude card. It never carries over or infers email, organization, plan, login method,
model-scoped quotas, Daily Routines/Cowork, extra usage, cost, or any other account-derived value. The card says the
available 5-hour/7-day data came from the user's own Claude Code statusLine configuration and that detailed limits are
unavailable with Keychain access disabled.

CodexBar does not apply observations to Admin API cards, explicit token accounts, claude-swap cards, selected or
non-Auto sources, or any multi-account presentation. When Keychain access is enabled, the feed is ignored because
Claude Code's statusLine payload has no account identity; OAuth, CLI, and Web continue normally. Direct Claude Code
Keychain access remains separately available, default-off, under its explicit consent toggle.

## Managed installation

In **Settings → Providers → Claude**, enable **Use your Claude Code statusLine feed** and choose **Install**. The
installed signed app writes this exact user-level command object, with the app's actual bundled helper path:

```json
{
  "statusLine": {
    "type": "command",
    "command": "'/Applications/CodexBar.app/Contents/Helpers/CodexBarCLI' claude statusline capture"
  }
}
```

Claude Code has one statusLine slot. CodexBar writes only `~/.claude/settings.json` (or the effective
`CLAUDE_CONFIG_DIR/settings.json`) and only when the slot is absent or matches the exact CodexBar-owned object. It
never edits project, local, or managed settings. An existing custom object is left untouched and the UI points to
manual composition below.

The install is atomic, preserves unrelated JSON and existing file permissions, rejects symbolic links, and refuses
malformed JSON or an unexpected root shape. If the app moves, Settings reports that the managed command needs repair;
it does not silently overwrite it. **Uninstall**, or turning the integration off, removes only an exact
CodexBar-owned object. A user-owned composition is merely disabled and is never edited. Manual deletion is an opt-out
and is never automatically reversed. The managed capture command intentionally writes no stdout, so it does not add
visible text to Claude Code's status line.

## Manual composition with an existing statusLine

Do not ask CodexBar to install over an existing command. Create a user-owned wrapper that reads stdin once, forwards
it to CodexBar, then forwards the same in-memory value to the command that renders your existing status line:

```sh
#!/bin/sh
input=$(cat)
printf '%s' "$input" | '/Applications/CodexBar.app/Contents/Helpers/CodexBarCLI' claude statusline capture
printf '%s' "$input" | "$HOME/.local/bin/my-existing-statusline"
```

Make the wrapper executable and configure it as Claude Code's `statusLine.command`. This remains a user-owned object:
CodexBar will not modify or uninstall it. Adjust the existing-command path and arguments to match your setup. The raw
payload stays in process memory and is not written by the example.

## Capture behavior

`codexbar claude statusline capture` reads at most 1 MiB from stdin, accepts one JSON object, and exits without stdout.
Malformed JSON, an observation with no valid window, stale or future-skewed observations, profile mismatch, missing
files, and upstream schema drift all mean "no observation." One valid window is still shown in its correct lane; an
invalid sibling or reset value is simply omitted. Missing data never becomes zero usage and never creates a Claude
provider error. The observation file is mode `0600` under a mode `0700` CodexBar Application Support directory. This
path contains no Security.framework item operation and possesses no OAuth token.
