-- repo: github.com/sst/opencode
-- commit: fe3f3a41f79ad292cc3c7c629567385a20ec5130  path: packages/core/src/database/schema.gen.ts  lines 181-190, 112-115
-- retrieved: 2026-09-22
      yield* tx.run(`
        CREATE TABLE \`session\` (
          \`id\` text PRIMARY KEY,
          \`project_id\` text NOT NULL,
          \`workspace_id\` text,
          \`parent_id\` text,
          \`slug\` text NOT NULL,
          \`directory\` text NOT NULL,
          \`path\` text,
          \`title\` text NOT NULL,
-- ---
        CREATE TABLE \`project\` (
          \`id\` text PRIMARY KEY,
          \`worktree\` text NOT NULL,
          \`vcs\` text,
