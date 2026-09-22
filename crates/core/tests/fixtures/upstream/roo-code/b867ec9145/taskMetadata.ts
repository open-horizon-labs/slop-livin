// repo: github.com/RooCodeInc/Roo-Code (Apache-2.0)
// commit: b867ec9145750d0ae1ff7f02d35406e9bf2a0b16  path: src/core/task-persistence/taskMetadata.ts
// retrieved: 2026-09-22  lines: 15-23, 108-115

export type TaskMetadataOptions = {
	taskId: string
	rootTaskId?: string
	parentTaskId?: string
	taskNumber: number
	messages: ClineMessage[]
	globalStoragePath: string
	workspace: string
	mode?: string
// ...
		cacheReads: tokenUsage.totalCacheReads,
		totalCost: tokenUsage.totalCost,
		size: taskDirSize,
		workspace,
		mode,
		...(typeof apiConfigName === "string" && apiConfigName.length > 0 ? { apiConfigName } : {}),
		...(initialStatus && { status: initialStatus }),
	}
