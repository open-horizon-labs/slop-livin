// repo: github.com/sst/opencode
// commit: fe3f3a41f79ad292cc3c7c629567385a20ec5130  path: packages/opencode/src/storage/storage.ts (Storage.migration.1 dest paths)
// retrieved: 2026-09-22
// L97-102 (legacy source glob + worktree from message field path.root):
        for (const msgFile of yield* fs.glob("storage/session/message/*/*.json", {
          cwd: full,
          absolute: true,
        })) {
          const json = decodeRoot(yield* fs.readJson(msgFile), { onExcessProperty: "preserve" })
          const root = Option.isSome(json) ? json.value.path?.root : undefined
// L120-127 (dest project/<id>.json with worktree):

        yield* fs.writeWithDirs(
          path.join(dir, "project", projectID + ".json"),
          JSON.stringify(
            {
              id,
              vcs: "git",
              worktree,
// L138-143 (legacy session/info/*.json -> dest session/<projectID>/<id>.json):
        yield* Effect.logInfo(`migrating sessions for project ${projectID}`)
        for (const sessionFile of yield* fs.glob("storage/session/info/*.json", {
          cwd: full,
          absolute: true,
        })) {
          const dest = path.join(dir, "session", projectID, path.basename(sessionFile))
// L149-153 (legacy session/message/<sid>/*.json -> dest message/<sid>/<mid>.json):
          yield* Effect.logInfo(`migrating messages for session ${info.value.id}`)
          for (const msgFile of yield* fs.glob(`storage/session/message/${info.value.id}/*.json`, {
            cwd: full,
            absolute: true,
          })) {
// L164-169 (legacy session/part/<sid>/<mid>/*.json -> dest part/<mid>/<pid>.json):
            yield* Effect.logInfo(`migrating parts for message ${item.value.id}`)
            for (const partFile of yield* fs.glob(`storage/session/part/${info.value.id}/${item.value.id}/*.json`, {
              cwd: full,
              absolute: true,
            })) {
              const out = path.join(dir, "part", item.value.id, path.basename(partFile))
