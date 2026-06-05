const BASE = "/api";

export interface SubmitResponse {
  job_id: string;
}

export type PollResponse =
  | { status: "pending" }
  | { status: "complete"; job_id: string; text: string; confidence: number };

export async function submitOcr(imageB64: string): Promise<SubmitResponse> {
  const res = await fetch(`${BASE}/ocr`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ image_b64: imageB64 }),
  });
  if (!res.ok) throw new Error(`submit failed: ${res.status}`);
  return res.json();
}

export async function pollOcr(jobId: string): Promise<PollResponse> {
  const res = await fetch(`${BASE}/ocr/${jobId}`);
  if (!res.ok) throw new Error(`poll failed: ${res.status}`);
  return res.json();
}

export function toBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result as string;
      // strip data URL prefix
      resolve(result.split(",")[1]);
    };
    reader.onerror = reject;
    reader.readAsDataURL(file);
  });
}
