# Publish Sky CUA through Gitea and deploy it to Saga

This ExecPlan is a living document governed by `~/.agents/PLANS.md` and
`plans/AGENTS.md`. Keep `Progress`, `Surprises & Discoveries`, `Decision Log`,
and `Outcomes & Retrospective` current while executing it.

This is a plan, not authorization to enable runners, add secrets, change
workflows or hosts, create releases, install artifacts, restart services, or
deploy.

## Purpose / Big Picture

Build Sky CUA on the local Asgard host through Gitea Actions, publish the
standalone Linux archive as a Gitea Release, and make Saga download, verify, and
install those exact bytes.

The trust assumption is deliberately simple: the Sky CUA repository, Asgard
runner account, and Saga administration path are trusted personal
infrastructure. The design protects correctness without adding hostile-workload
isolation.

The boundaries are:

1. One trusted release job builds, tests, and publishes the archive.
2. The Gitea Release is the durable producer-to-target handoff.
3. A separate workflow can deploy or redeploy an existing tag and SHA-256.
4. Saga owns consumer stop/start order and target health.

The installer remains release-agnostic:

    python3 install.py build
    tar xzf sky-cua-linux-x64-glibc.tar.gz
    cd sky-cua-linux-x64-glibc
    python3 install.py install

On Saga, installation directly replaces
`/home/ubuntu/.local/share/sky-cua`. Do not add generations, `current`,
retained rollback, activation receipts, installer hash arguments, staging
promotion, backups, or a deployment state machine.

## Progress

- [x] Confirm the standalone build/install contract.
- [x] Inspect live Gitea 1.25.4, repository, runners, releases, and secrets.
- [x] Inspect Saga over the existing SSH route.
- [x] Simplify the design for trusted personal infrastructure.
- [ ] Resolve the first-cutover consumer and deterministic health commands.
- [ ] Prove the Gitea release-attachment sequence with a tiny canary.
- [ ] Enable `asgard-build-1` at capacity one and repair verification.
- [ ] Implement manual release publication and retryable Saga deployment.
- [ ] Implement Saga's fixed-root hook and health gate.
- [ ] Prove the first cutover and same-identity reinstall.
- [ ] Add automatic `standalone-v*` tag publication.

## Surprises & Discoveries

- The primary repository is public `bex/sky-cua` on
  `https://git.heliasar.com`, not GitHub. Gitea recognizes
  `.github/workflows/verify.yml`, but its `ubuntu-latest` jobs currently have no
  online matching runner.

- `asgard-build-1` and `asgard-build-2` are registered Gitea runners. Both user
  services are disabled/inactive. Sky CUA needs only non-privileged
  `asgard-build-1`; `asgard-build-2` advertises `privileged-build`.

- The repository has no releases or Actions artifacts. Its only existing
  secret, `HELIASAR_MARKETPLACE_TOKEN`, is unrelated and must not be reused.

- Gitea 1.25.4 supports release attachments and API workflow dispatch, but
  ignores workflow `concurrency`, `permissions`, `environment`, and
  `timeout-minutes`. Capacity one on Asgard and a Saga-local `flock` cover the
  relevant concurrency gap.

- Gitea attachments are mutable and expose no server-computed digest. The
  release job must read the uploaded archive back and hash it. The independently
  dispatched SHA-256 is authoritative; the `.sha256` attachment is for operator
  convenience.

- Saga can directly download public Sky CUA releases from Gitea.

- Saga currently has `computer-use` and `browser-use`; current Sky CUA produces
  `computer-use` and `browser`. Saga's current OpenClaw also expects
  `browser-use`. This is a first-deployment compatibility blocker.

- Saga's `/opt/homelab/ops/src/saga_ops/sky_cua.py` and
  `ops/tests/test-sky-cua-transaction.sh` implement the retired
  generations/current/rollback model. Replace or delete that model.

## Decision Log

- **Gitea owns the pipeline.** Put workflows in `.gitea/workflows/` and retire
  `.github/workflows/verify.yml` so the GitHub mirror is not a second owner.

- **Use three workflows:** `verify.yml`, `release-standalone.yml`, and
  `deploy-saga.yml`.

- **Build and publish in one job on `asgard-build-1`.** Do not upload/download
  an intermediate Actions artifact or claim cross-job credential isolation.

- **Keep deployment separate for retries.** Its entire artifact interface is
  `tag` plus `archive_sha256`; it checks out no release source and builds
  nothing.

- **Dispatch deployment from `main`.** The release tag identifies bytes but
  does not select the deployment workflow definition.

- **Publish only the archive and `.sha256` sidecar.** The archive already
  contains release metadata; put the source commit in the Gitea Release
  description. Do not add a pipeline metadata JSON.

- **Use the existing `ssh saga` route** if it works inside a runner job. Add no
  dedicated deployment key, forced command, SSH-key secret, or parallel host-key
  scheme unless live behavior requires one.

- **Use one practical secret:** `SKY_CUA_GITEA_TOKEN`, with repository write
  access sufficient for releases, attachments, and deployment dispatch.

- **Roll out manually first.** Add the tag trigger only after the first cutover
  and same-identity reinstall pass. Do not add an `AUTO_DEPLOY` flag.

- **Routine health is deterministic.** Full model-backed Computer Use and
  Browser Use acceptance belongs to the first cutover and intentional manual
  release testing, not every automatic deployment.

## Context and Orientation

### Producer and Asgard

`python3 install.py build` in `/home/bex/projects/sky-cua` emits:

    dist/sky-cua-linux-x64-glibc.tar.gz

The extracted archive installs through its own `python3 install.py install`.
The current product is `sky-cua`, version `0.1.0`, target
`linux-x64-glibc`. Derive version from source/artifact metadata rather than
duplicating a permanent YAML constant.

Gitea is `https://git.heliasar.com` version 1.25.4. Repository
`bex/sky-cua` has Actions and Releases enabled. Registered
`asgard-build-1` is offline because its existing user service is disabled. The
host has 32 CPUs, about 62 GiB RAM, and about 118 GiB free; existing generated
trees use roughly 56 GiB.

Run `asgard-build-1` at capacity one with a free-space preflight and ordinary
workspace cleanup. Use `checkout@v4` or a proven specific v4 release; this
trusted setup does not need commit-SHA pinning for every action.

### Saga

- SSH alias: `saga`, `ubuntu@51.178.30.94:22`, `IdentitiesOnly yes`.
- Confirmed ED25519 host-key fingerprint:
  `SHA256:ltyy8lAWePi1T65HnOjPKptpAQvmvwoa5ZXek9ztsWA`.
- Ubuntu 26.04, x86_64, glibc 2.43, Python 3.14.4.
- Root filesystem has about 22 GiB free; `/tmp` is a 4.5 GiB tmpfs.
- Install owner is `ubuntu`; `XDG_DATA_HOME` is unset.
- Existing fixed-root install is about 471 MiB and reports
  `sky-cua 0.1.0 linux-x64-glibc`.
- User service `openclaw-gateway.service` is active and
  `openclaw gateway status` succeeds.
- System service `brave-origin.service` runs as `ubuntu`.
- `/opt/homelab/scripts/sky-cua-xpra-mcp.sh` already uses the fixed Sky CUA
  root and Saga's real display, D-Bus, XDG, service, and socket paths.
- Saga operations are owned by `/opt/homelab`.

## Artifact and Workflow Interfaces

Release tag:

    standalone-v<VERSION>

Release attachments:

    sky-cua-linux-x64-glibc.tar.gz
    sky-cua-linux-x64-glibc.tar.gz.sha256

Deployment workflow inputs:

    tag=standalone-v<VERSION>
    archive_sha256=<64 lowercase hex characters>

Before publication, verify that the tag points to the checked-out commit and
that tag version, embedded version, product, and target agree. Record the full
source commit in the release description and Actions log.

After upload, download the public archive to a fresh path and confirm its raw
SHA-256. Dispatch:

    POST /api/v1/repos/bex/sky-cua/actions/workflows/deploy-saga.yml/dispatches

with `ref: main` and the two inputs above. Saga downloads the archive from the
public release URL and repeats the digest check. The digest guards transport
identity outside `install.py`; it is not an installer argument.

If a retry finds an existing release, continue only when its downloaded archive
matches the expected digest and source tag. Otherwise fail and use a new tag.
Never overwrite different bytes under an existing tag.

## Plan of Work

### Phase 1: prerequisites

Deploy the OpenClaw consumer that expects `computer-use` and `browser`. During
the coordinated first cutover, remove installed `browser-use` through the
supported Codex plugin command. Do not make this a recurring deployment step.

Define one Saga deterministic health command covering installed identity,
stable paths, native-host/session readiness, and both service owners without an
external model call.

Review and enable `asgard-build-1`, then prove:

- a no-secret job reaches `runs-on: asgard-build-1`;
- job workspace cleanup and build free-space failure work;
- `ssh saga` is noninteractive inside the actual job environment.

Move verification to `.gitea/workflows/verify.yml`, select
`asgard-build-1`, preserve existing Rust/Python checks, and remove
`.github/workflows/verify.yml`. Packaging remains release-only.

### Phase 2: Gitea release canary

Run one explicitly disposable canary:

1. Create a draft release and upload a known small file.
2. Attempt authenticated attachment readback while draft.
3. Publish, then perform anonymous readback and compare bytes.
4. Delete the disposable release as the planned canary cleanup.

Use the proven API order in production. If draft readback is unavailable,
publish first, read back immediately, and refuse deployment on mismatch. Do not
overwrite a bad published attachment.

### Phase 3: manual release workflow

Add `.gitea/workflows/release-standalone.yml` with only
`workflow_dispatch` and a required tag input. Its single trusted job:

1. Validate and check out the requested `standalone-v*` tag.
2. Confirm tag version, source version, target, and source commit.
3. Run the existing release verification gate.
4. Record a clean checkout.
5. Run `python3 install.py build`.
6. Fail if packaging changed tracked source.
7. Validate archive safety and expected shape.
8. Run the extracted installer in an isolated environment.
9. Compute SHA-256 and write the sidecar.
10. Create or reuse the Gitea Release according to the retry contract.
11. Upload both attachments without overwrite.
12. Read back and hash the published archive.
13. Dispatch `deploy-saga.yml` from `main` with tag and digest.

The isolated installer smoke sets:

    HOME=<temporary home>
    XDG_DATA_HOME=<temporary home>/.local/share
    XDG_CONFIG_HOME=<temporary home>/.config
    XDG_CACHE_HOME=<temporary home>/.cache
    PATH=<controlled system tool path>

Exclude real Codex/OpenClaw executables, or provide harmless test doubles only
when their detection path needs coverage. The smoke must not touch the runner
user's real configuration.

### Phase 4: Saga deployment hook

Replace the obsolete Saga transaction with one narrow entrypoint:

    deploy-sky-cua <standalone-tag> <sha256>

Use Bash while it remains small and linear. If JSON/archive validation and
cleanup make shell quoting dominant, replace the obsolete Python command with
one small direct-deploy implementation and focused tests. Do not build a
deployment framework or transaction abstraction.

The entrypoint:

1. Validates tag and digest shapes and acquires a nonblocking local `flock`.
2. Preflights disk space, downloads the public archive, and verifies SHA-256.
3. Validates archive paths/links and embedded product, target, and tag version.
4. Extracts into an ephemeral directory.
5. Stops `openclaw-gateway.service`, then `brave-origin.service`.
6. Runs extracted `python3 install.py install` as `ubuntu` with explicit
   Saga HOME/XDG values.
7. Starts `brave-origin.service` and waits for Xpra/session/native-host
   readiness.
8. Starts `openclaw-gateway.service`.
9. Runs deterministic health and reports installed identity/service results.

Download, hash, inspect, and extract before stopping services. OpenClaw stops
first and starts last so it cannot use a disappearing or unready browser
runtime.

The deploy workflow validates its two inputs and invokes this command through
the existing `ssh saga` route. It checks out no release source.

### Phase 5: rollout

1. Prove Gitea verification and the release canary.
2. Complete the OpenClaw/plugin-name prerequisite.
3. Add `SKY_CUA_GITEA_TOKEN`.
4. Manually dispatch the first release.
5. Observe build, isolated install, publication readback, Saga install,
   consumer sequencing, and deterministic health as separate gates.
6. Run full live acceptance: Computer Use on the intended session and
   external-native-host Browser Use, intended provider/model, no fallback.
7. Redispatch the same tag and digest; prove convergence and no stale consumer
   processes.
8. Add `push.tags: ["standalone-v*"]` in a final small change.

## Archive Safety

Before any `tar xzf`, producer smoke and Saga must reject:

- absolute paths or path components containing `..`;
- multiple or unexpected top-level roots;
- symlinks or hard links escaping the extracted tree;
- missing installer or embedded release metadata;
- missing expected runtime, plugins, skills, or native-host content.

Use one small Python `tarfile` validator where practical and test malformed
fixtures. This prevents packaging mistakes from escaping the extraction
directory; it is not a new artifact framework.

## Deterministic Health Contract

Every automatic deployment proves:

- installed version/target and stable launchers match the release;
- native manifests point to the stable host launcher;
- canonical `computer-use` and `browser` plugins and skills exist;
- retired `browser-use` is absent or disabled after the first cutover;
- `brave-origin.service` and `openclaw-gateway.service` are active with new
  processes;
- Xpra/session and local native-host readiness pass;
- `openclaw gateway status` succeeds;
- OpenClaw reports the canonical managed plugin owners.

The exact native-host/session and managed-plugin commands remain to be derived
from current Saga behavior. Promote a cheap local CUA/Browser probe only after
it is deterministic. External provider availability must not decide routine
deployment success.

## Failure and Retry Behavior

- Pre-publication failure publishes and dispatches nothing.
- Existing release bytes must match before a retry continues.
- Publication readback mismatch fails without Saga dispatch.
- Dispatch failure is retried for the same published tag and digest without
  rebuilding.
- Saga disk, lock, download, digest, metadata, or archive failure stops before
  service downtime.
- Install, restart, or health failure remains failed; there is no rollback.
  Inspect live state, correct the cause, and rerun the same tag/digest.
- Do not add automatic retry loops without evidence for a specific transient
  operation.

## Validation

Implementation is complete when:

- single-capacity `asgard-build-1` owns verify/release jobs;
- packaging leaves tracked source clean;
- archive safety and isolated extracted installation pass;
- Gitea exposes exactly the archive and sidecar;
- fresh readback matches producer SHA-256;
- deploy dispatch uses workflow `main`;
- Saga repeats the digest check and installs as `ubuntu`;
- consumers stop before replacement and start in dependency order;
- deterministic health passes;
- first-cutover full CUA/Browser acceptance passes;
- same-identity redispatch converges without stale processes;
- digest mismatch fails closed;
- only then does automatic tag publication land.

For this documentation revision, validation is limited to structure,
diff/whitespace inspection, and confirmation that no other repository file
changed. No runner, release, host, install, restart, or live gate is run.

## Idempotence and Recovery

Publication is create-once by tag/attachment name. Direct fixed-root
installation is repeatable with the same archive. Runner capacity one
serializes builds; Saga `flock` serializes deployments.

Recovery is forward-only: redispatch the same confirmed identity after fixing an
operational problem, or publish a new tag for changed bytes. Do not retain
generations, backups, rollback, or mutate an existing release.

## Artifacts

Durable artifacts are the archive and `.sha256` attachment. Gitea retains
tag/source-commit context and workflow logs. Saga logs requested identity,
installed release, service sequencing, and health. Add no second provenance
manifest until a real consumer needs it.

## Interfaces and Files Likely Affected

Sky CUA:

- replace `.github/workflows/verify.yml` with `.gitea/workflows/verify.yml`;
- add `.gitea/workflows/release-standalone.yml`;
- add `.gitea/workflows/deploy-saga.yml`;
- add a small archive validator only if existing code lacks the seam;
- add focused tag, archive, release-retry, and dispatch tests;
- update `ROADMAP.md` and operations documentation when shipped.

Saga `/opt/homelab`:

- add a narrow deployment entrypoint;
- replace/delete `ops/src/saga_ops/sky_cua.py` and its transaction tests;
- preserve and verify `scripts/sky-cua-xpra-mcp.sh`;
- use existing Brave and OpenClaw service ownership;
- test archive rejection, ordering, convergence, and health.

Configuration:

- enable `gitea-act-runner-asgard-build-1.service` at capacity one;
- establish practical cleanup/free-space behavior;
- add `SKY_CUA_GITEA_TOKEN`;
- use existing runner-account `ssh saga`;
- perform the one-time OpenClaw/`browser-use` cutover.

## Remaining Unknowns to Resolve

- Exact deterministic Xpra/session, native-host, and managed-plugin checks.
- Whether existing SSH/sudo access can restart `brave-origin.service`.
- Whether `ssh saga` is noninteractive inside a real runner job.
- Live Gitea draft/authenticated/public attachment-readback behavior.
- Saga download directory and free-space threshold.
- Exact first-cutover OpenClaw version and supported `browser-use` removal
  command.

## Outcomes & Retrospective

No pipeline or external state changed. The revised plan retains exact published
bytes, retryable deployment, safe extraction, target locking, consumer
sequencing, and deterministic health while removing intermediate Actions
artifacts, metadata schema, dedicated SSH authority, credential partitioning,
and routine model-backed acceptance.
