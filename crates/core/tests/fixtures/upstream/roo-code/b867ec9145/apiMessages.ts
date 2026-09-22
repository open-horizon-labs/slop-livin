// repo: github.com/RooCodeInc/Roo-Code (Apache-2.0)
// commit: b867ec9145750d0ae1ff7f02d35406e9bf2a0b16  path: src/core/task-persistence/apiMessages.ts
// retrieved: 2026-09-22  lines: 40-48, 118-119

export async function readApiMessages({
	taskId,
	globalStoragePath,
}: {
	taskId: string
	globalStoragePath: string
}): Promise<ApiMessage[]> {
	const taskDir = await getTaskDirectoryPath(globalStoragePath, taskId)
	const filePath = path.join(taskDir, GlobalFileNames.apiConversationHistory)
// ...
	const taskDir = await getTaskDirectoryPath(globalStoragePath, taskId)
	const filePath = path.join(taskDir, GlobalFileNames.apiConversationHistory)
