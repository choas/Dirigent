# Things to check — herdr backend

The Herdr provider (`src/herdr/`, see `docs/How_to_Herdr.md`) runs native Claude
Code in a herdr pane. Unit tests cover the decision logic against a scripted herdr,
and `cargo test herdr_smoke -- --ignored --nocapture` covers one real run. The
rest needs a person at the keyboard.

## Manual test checklist

Start herdr first. Pick **Herdr** in Settings → General, or use the **H** button
in the prompt field.

1. **Test Connection** shows a ✓ reply. (Only if nothing else is running in
   herdr: with its server stopped, Test Connection says it is not running.)
2. **Happy path**: a cue that edits a file. A new herdr tab opens and focuses,
   Claude starts, the prompt arrives as one message (multi-line too), Claude
   works. When it stops, the cue moves to Review with the diff, and the
   conversation shows `You: … / Claude: …` with the trailing
   `<commit-message>` block. Turns and tokens are filled in; cost is empty.
3. **Reply** to that cue: a new tab starts with `--resume <same id>`, only the
   reply text is sent, and the conversation continues.
4. **Cancel while Claude is working**: Claude stops and exits in the pane (the
   tab stays open). The running log says
   `Cancelled: interrupted Claude Code … it exited`.
5. **Pane closed mid-run** (close the tab in herdr): the run fails with
   "the herdr pane was closed before the Stop hook fired".
6. **Claude exits mid-run** (`/exit` in the pane, or Ctrl-C twice): the run
   fails with "Claude Code exited in the herdr pane before the Stop hook fired".
7. **Malformed `.claude/settings.local.json`** (e.g. `{oops`): the run fails at
   once with "cannot install the Claude Code Stop hook …", the file is left as
   it was, and no tab opens.
8. **Bad Herdr CLI Path** in Settings: the run fails naming that path; it
   does not fall back to `herdr` on PATH.
9. **Env forwarding**: put `HERDR_CHECK=1` in `.Dirigent/.env` and ask Claude
   to run `echo $HERDR_CHECK`. It prints `1`, and `ps aux | grep HERDR_CHECK`
   shows nothing while the run starts.
10. **New / untrusted folder**: Claude shows its folder-trust dialog. The
    running log says to answer it in the pane; after you answer, the prompt is
    sent.
11. **Dirigent MCP**: a prompt that mentions "Dirigent" starts Claude with
    `--mcp-config …` in the pane's command line.

## Open questions

- **Old tab on reply.** A reply resumes the session in a new tab while the
  previous tab's Claude is still idle on the same session. Should Dirigent
  reuse the old pane when it is alive (send the reply there), or close it?
- **Folder trust.** The PTY path auto-accepts Claude's trust dialog for
  repositories approved in Dirigent's trust gate. Herdr leaves it to the user
  in the pane. Is that the right call?
- **Live progress.** The running log only shows lifecycle lines. herdr's
  `pane.read` or event subscriptions could stream a preview.
- **Transcript format.** It is observable, not a public contract. If Claude
  Code changes it, the response falls back to the pane text.
