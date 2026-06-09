import { useState, useEffect, useRef } from "react";
import { submitOcr, pollOcr, compressAndEncode, PollResponse } from "./api";

type JobState =
  | { phase: "idle" }
  | { phase: "submitting" }
  | { phase: "polling"; jobId: string }
  | { phase: "complete"; result: PollResponse & { status: "complete" } }
  | { phase: "error"; message: string };

const POLL_INTERVAL_MS = 1500;

export function useOcrJob() {
  const [state, setState] = useState<JobState>({ phase: "idle" });
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const cancelPoll = () => {
    if (timeoutRef.current !== null) {
      clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
  };

  useEffect(() => () => cancelPoll(), []);

  // Recursive setTimeout instead of setInterval — the next poll is only
  // scheduled after the previous response lands, preventing two requests
  // from racing and the second one getting a 404 on an already-consumed result.
  const schedulePoll = (jobId: string) => {
    timeoutRef.current = setTimeout(async () => {
      try {
        const res = await pollOcr(jobId);
        if (res.status === "complete") {
          setState({ phase: "complete", result: res });
        } else if (res.status === "failed") {
          setState({ phase: "error", message: res.error });
        } else {
          schedulePoll(jobId); // still pending — reschedule
        }
      } catch (err) {
        setState({ phase: "error", message: String(err) });
      }
    }, POLL_INTERVAL_MS);
  };

  const submit = async (file: File) => {
    setState({ phase: "submitting" });
    try {
      const b64 = await compressAndEncode(file);
      const { job_id } = await submitOcr(b64);
      setState({ phase: "polling", jobId: job_id });
      schedulePoll(job_id);
    } catch (err) {
      setState({ phase: "error", message: String(err) });
    }
  };

  const reset = () => {
    cancelPoll();
    setState({ phase: "idle" });
  };

  return { state, submit, reset };
}
