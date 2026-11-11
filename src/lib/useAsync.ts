import { useEffect, useState } from "react";
import { errorMessage } from "../api";

/**
 * Loads data whenever `deps` change and ignores answers that arrive after a newer request or
 * after unmount. On failure `data` is cleared and `error` holds the message.
 */
export function useAsync<T>(load: () => Promise<T>, deps: unknown[]): { data: T | null; error: string | null } {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    load()
      .then((d) => {
        if (alive) {
          setData(d);
          setError(null);
        }
      })
      .catch((e) => {
        if (alive) {
          setData(null);
          setError(errorMessage(e));
        }
      });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return { data, error };
}
