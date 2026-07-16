# Explicit git closeout

Load only when the user explicitly requests a commit and/or push.

Inspect status and the relevant diff first. Preserve unrelated worktree
changes and stage only files relevant to the request. Use the `committer`
subagent for an explicit commit or commit-plus-push request when available; it
handles precise staging and a diff-grounded semantic message.

- Explicit commit wording authorizes a commit only.
- Explicit push wording authorizes a push only; push is an external write.
- If both are explicitly requested, commit first and push the resulting commit
  only after the commit succeeds.
- A deploy, build, package, install, restart, “ship”, or “release” request does
  not add either git action. Do not restore automatic git behavior.

If a user explicitly sequences git closeout after a deploy or package step,
honor that dependency and stop before git actions when the prerequisite fails.
Report the staged scope, commit identifier, and push result when applicable.
