// repo: github.com/google-gemini/gemini-cli
// commit: d5b3e3accb26000d273abf16e0f1dd83aa5428a9  path: packages/core/src/config/storage.ts lines 287-324
// retrieved: 2026-09-22  (projects.json registry = current; sha256 hash dirs = legacy)
      await Storage.ensureGlobalRuntimeDirExists();

      const registryPath = path.join(
        Storage.getGlobalRuntimeDir(),
        'projects.json',
      );
      const registry = new ProjectRegistry(registryPath, [
        Storage.getGlobalTempDir(),
        path.join(Storage.getGlobalRuntimeDir(), 'history'),
      ]);
      await registry.initialize();

      this.projectIdentifier = await registry.getShortId(this.getProjectRoot());
      await this.performMigration();
    })();

    return this.initPromise;
  }

  /**
   * Performs migration of legacy hash-based directories to the new slug-based format.
   * This is called internally by initialize().
   */
  private async performMigration(): Promise<void> {
    const shortId = this.getProjectIdentifier();
    const oldHash = this.getFilePathHash(this.getProjectRoot());

    // Migrate Temp Dir
    const newTempDir = path.join(Storage.getGlobalTempDir(), shortId);
    const oldTempDir = path.join(Storage.getGlobalTempDir(), oldHash);
    await StorageMigration.migrateDirectory(oldTempDir, newTempDir);

    // Migrate History Dir
    const historyDir = path.join(Storage.getGlobalRuntimeDir(), 'history');
    const newHistoryDir = path.join(historyDir, shortId);
    const oldHistoryDir = path.join(historyDir, oldHash);
    await StorageMigration.migrateDirectory(oldHistoryDir, newHistoryDir);
  }
