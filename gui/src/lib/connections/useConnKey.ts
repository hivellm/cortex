/**
 * useConnKey — phase3 §4.
 *
 * Every TanStack Query `useQuery` call in the renderer needs to be
 * scoped to the active connection so switching backends does not
 * surface cached results from the previous one. The hook returns
 * the active connection id; call sites build their queryKey as
 * `[connKey, ...rest]`. When the user switches connection, the
 * key changes, the cached data for the old connection stays warm
 * (per spec scenario "fast switch-back"), and the new connection
 * fetches fresh.
 *
 * Pattern:
 *
 *   const connKey = useConnKey();
 *   useQuery({
 *     queryKey: [connKey, "overview"],
 *     queryFn: () => api.overview(),
 *   });
 *
 * Naming: `connKey` (not `connectionId`) so the prefix slot is
 * grep-able across the codebase — every queryKey starting with
 * `[connKey, ...]` is automatically scoped.
 */

import { useActiveConnection } from "./store";

export function useConnKey(): string {
  return useActiveConnection().id;
}
