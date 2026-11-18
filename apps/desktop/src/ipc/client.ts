import { invoke } from "@tauri-apps/api/core";
export type Reply<T> = { ok: boolean; data: T; error?: string };
export type Operation = { id: string; status: string; error: string | null };
export async function request<T>(method: string, params?: object): Promise<T> {
  const response = await invoke<Reply<T>>("request", {
    command: { method, ...(params ? { params } : {}) },
  }).catch((error) => {
    throw new Error(String(error));
  });
  if (!response.ok) throw new Error(response.error || "Daemon request failed");
  return response.data;
}
export async function waitOperation(id: string): Promise<Operation> {
  for (let count = 0; count < 2400; count++) {
    const operation = await request<Operation>("operation", { id });
    if (operation.status === "completed") return operation;
    if (operation.status !== "running")
      throw new Error(operation.error || operation.status);
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error(
    `Operation ${id} is still running; reconnect to check its outcome`,
  );
}
