import { vi } from "vitest";
import { type Method, setBackend } from "./api";

/** Answers every call with what `handler` returns (or throws), in place of the Glint worker; returns the recorder. */
export function useBackend(handler: (method: Method, args: unknown[]) => unknown) {
  const call = vi.fn(async (method: Method, args: unknown[], _signal?: AbortSignal) => handler(method, args));
  setBackend({ call });
  return call;
}
