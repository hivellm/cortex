import { createContext, useContext } from "react";

import type { Filters } from "./api";

/// Empty filter set — no session, no repo, no kind. Used as the
/// default context value and as the "clear filters" target.
export const EMPTY_FILTERS: Filters = {};

export type FiltersContextValue = {
  filters: Filters;
  setFilters: (f: Filters) => void;
  setFilter: <K extends keyof Filters>(key: K, value: Filters[K] | undefined) => void;
  clearFilters: () => void;
};

export const FiltersContext = createContext<FiltersContextValue>({
  filters: EMPTY_FILTERS,
  setFilters: () => {},
  setFilter: () => {},
  clearFilters: () => {},
});

export function useFilters(): FiltersContextValue {
  return useContext(FiltersContext);
}

/// `true` when the filter object has at least one active key.
export function hasAnyFilter(f: Filters): boolean {
  return Boolean(
    f.session_id || (f.repo && f.repo.length > 0) || f.kind || f.content_hash,
  );
}
