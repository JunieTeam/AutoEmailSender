import { MutationCache, QueryCache, QueryClient } from "@tanstack/react-query";
import { logUnexplained } from "./ui-errors";

// Everything goes to the local core: retrying at once rarely helps and only delays the error, and
// whether the computer is online says nothing about whether a call can run.
export const queryClient = new QueryClient({
  queryCache: new QueryCache({ onError: logUnexplained }),
  mutationCache: new MutationCache({ onError: logUnexplained }),
  defaultOptions: {
    queries: { retry: false, networkMode: "always" },
    mutations: { networkMode: "always" },
  },
});
