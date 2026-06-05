import { useState, useEffect, useRef } from "react";
import { submitOcr, pollOcr, toBase64, PollResponse } from "./api";

type JobState =
  | { phase: "idle" }
  | { phase: "submitting" }
  | { phase: "polling"; jobId: string }
  | { phase: "complete"; result: PollResponse & { status: "complete" } }
  | { phase: "error"; message: string };

const POLL_INTERVAL_MS = 1500;

export function useOcrJob() {
  const [state, setState] = useState<JobState>({ phase: "idle" });
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const clearPoller = () => {
    if (intervalRef.current !== null) {
      clearInterval(intervalRef.current);
      intervalRef.current = null;
    }
  };

  useEffect(() => () => clearPoller(), []);

  const submit = async (file: File) => {
    setState({ phase: "submitting" });
    try {
      const b64 = await toBase64(file);
      const { job_id } = await submitOcr(b64);
      setState({ phase: "polling", jobId: job_id });

      intervalRef.current = setInterval(async () => {
        try {
          const res = await pollOcr(job_id);
          if (res.status === "complete") {
            clearPoller();
            setState({ phase: "complete", result: res });
          }
        } catch (err) {
          clearPoller();
          setState({ phase: "error", message: String(err) });
        }
      }, POLL_INTERVAL_MS);
    } catch (err) {
      setState({ phase: "error", message: String(err) });
    }
  };

  const reset = () => {
    clearPoller();
    setState({ phase: "idle" });
  };

  return { state, submit, reset };
}
