# Plans Guide

`plans/` contains active forward-looking ExecPlans only. Each plan covers
one seam or feature area and is retired the moment the work it describes
lands and is proven. Plans are guidance, not authority: current source and
live runtime proof win.

## Conventions

- One ExecPlan per seam or feature area; human-readable slug filenames
  (`wayland_fallback_vision_anchors.md` is the pattern to follow).
- Use the standard ExecPlan shape: Purpose, Progress, Surprises &
  Discoveries, Decision Log, Outcomes & Retrospective, plus Context, Plan of
  Work, Validation, Idempotence, Artifacts, and Interfaces sections.
- Include exact source paths, commands, and artifacts that prove current
  state; separate confirmed behavior from proposed work.
- A plan explains a design seam; it does not track every session turn,
  duplicate `ROADMAP.md` entries, or store transcripts/raw JSON (point to
  artifacts and summarize).
- Do not use the `goals/<name>/` Plannotator package layout; ExecPlans are
  the single forward-looking format.

## Lifecycle

When an ExecPlan reaches "code complete" with live proof, retire it — do not
leave it as a postmortem or annotate it with a status line:

1. Create or update `docs/features/<slug>.md` from the template in
   `docs/AGENTS.md` (descriptive only; no Progress/Surprises/Decision Log).
2. Extract durable research into `docs/research/YYYY-MM-<slug>.md`, one
   question per file.
3. Update the matching `ROADMAP.md` entry: check the box, link the feature
   doc, add follow-up sub-items if partial work remains.
4. Delete the ExecPlan. Git history is the canonical archive; no
   `archive/plans/` shadow tree.
