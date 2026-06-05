import { useRef } from "react";
import { useOcrJob } from "./useOcrJob";

export default function App() {
  const { state, submit, reset } = useOcrJob();
  const inputRef = useRef<HTMLInputElement>(null);

  const handleFile = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (file) submit(file);
  };

  return (
    <main style={{ maxWidth: 640, margin: "4rem auto", fontFamily: "sans-serif" }}>
      <h1>Chiron OCR</h1>

      {state.phase === "idle" && (
        <>
          <input
            ref={inputRef}
            type="file"
            accept="image/*"
            style={{ display: "none" }}
            onChange={handleFile}
          />
          <button onClick={() => inputRef.current?.click()}>
            Upload image
          </button>
        </>
      )}

      {state.phase === "submitting" && <p>Submitting…</p>}

      {state.phase === "polling" && (
        <p>Processing job <code>{state.jobId}</code>…</p>
      )}

      {state.phase === "complete" && (
        <div>
          <h2>Result</h2>
          <pre
            style={{
              background: "#f4f4f4",
              padding: "1rem",
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
            }}
          >
            {state.result.text}
          </pre>
          <p>Confidence: {(state.result.confidence * 100).toFixed(1)}%</p>
          <button onClick={reset}>Try another</button>
        </div>
      )}

      {state.phase === "error" && (
        <div>
          <p style={{ color: "red" }}>Error: {state.message}</p>
          <button onClick={reset}>Try again</button>
        </div>
      )}
    </main>
  );
}
