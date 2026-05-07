# Result Channels: Git And Object Storage

## Purpose

Workers need a durable way to tell the coordinator where their work landed. Small structured summaries can stay in `AgentRunResult`, but code changes and large artifacts should move through external result channels:

- git branches and worktrees for code, diffs, commits, and PR-ready work;
- S3-compatible object storage for large reports, simulation outputs, datasets, screenshots, traces, and binary bundles.

The coordinator stores references. It does not copy full diffs, credentials, or large blobs into workflow state.

## Git Channel

`ExecutionProfile.results.git` defines the expected git path:

- `enabled`: whether the task should produce a git result;
- `remote`: usually `origin`;
- `base_ref`: branch or commit to base the worktree on;
- `branch_prefix`: default `coat/task`;
- `worktree_root`: runner-local root such as `/worktrees`;
- `push_on_success`: whether a runner should push after validation-ready work;
- `require_clean_diff`: whether the runner should fail if unrelated local changes are present;
- `include_patch_artifact`: whether to also emit a patch artifact.

Workers return `AgentRunResult.git_result` with:

- branch;
- worktree path;
- commit when created;
- push status;
- optional PR URL;
- optional diff URI.

The coordinator and validator treat `git_result` as artifact evidence. Reviewers and unifiers can use the branch/commit to inspect results without trusting the worker summary.

`coat-sandbox-runner` keeps live worktree creation behind an explicit local gate:

- `SANDBOX_ENABLE_LIVE_GIT_WORKTREES=true`;
- `SANDBOX_APPROVED_GIT_REPO_ROOTS=/repo/root,/another/repo/root`;
- `SANDBOX_REQUIRE_LIVE_GIT_WORKTREE_APPROVAL=true` by default;
- request payload `live_git_worktree.enabled=true`;
- request payload `live_git_worktree.approval_id` when approval is required.

When any gate is missing, the runner still records the planned git branch/worktree ref and adds a warning to the attestation instead of mutating a repository. When all gates pass, it runs `git worktree add` with argv-safe process execution, only for repos under approved roots, and returns `GitWorktree` evidence.

## Object Storage Channel

`ExecutionProfile.results.object_storage` defines the large-artifact path:

- `store`: bucket, endpoint, region, path-style flag, and `SecretRef`;
- `key_prefix_template`: default `goals/{goal_id}/tasks/{task_id}`;
- `require_manifest`: require an `artifact-manifest.json` object for multi-file outputs;
- `max_inline_bytes`: threshold above which outputs must leave workflow state.

Workers return `AgentRunResult.object_artifacts`, each with store, key, URI, content type, size, hash, and description.

Local Compose runs a MinIO S3-compatible service as `object-store` and initializes the `coat-artifacts` bucket. Kubernetes includes the same development object-store Deployment and Job. In AWS/EKS, use real S3 by setting the object store endpoint/region/bucket and resolving credentials through IAM roles for service accounts or another `SecretRef` provider.

## Runner Rules

- Never write raw object-store credentials into task state.
- Prefer git for source changes and object storage for large generated outputs.
- Use one branch per task unless a unifier explicitly joins branches.
- Branch names should include goal ID and task ID.
- Object keys should include goal ID and task ID.
- Workers must return structured refs even when they also include human-readable summaries.
- Reviewers should inspect git/object refs directly when deciding satisfaction.

## Local Defaults

Compose:

- S3 endpoint: `http://object-store:9000`
- bucket: `coat-artifacts`
- access key: `coat`
- secret key: `coat-local-secret`

These defaults are for local development only. Production should use cloud object storage, workload identity, or a managed secret source.
