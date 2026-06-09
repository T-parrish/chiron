const BASE = "/api";

export interface SubmitResponse {
  job_id: string;
}

export type PollResponse =
  | { status: "pending" }
  | { status: "complete"; job_id: string; text: string; confidence: number }
  | { status: "failed"; job_id: string; error: string };

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

// ---------------------------------------------------------------------------
// Image compression
// ---------------------------------------------------------------------------

const MAX_DIMENSION = 2048; // px — enough resolution for OCR, kills needless size
const JPEG_QUALITY = 0.88;  // 0–1; 0.88 is visually lossless for text

/**
 * Resize (if needed) and re-encode the image as JPEG before base64-encoding.
 * Keeps aspect ratio; only downscales, never upscales.
 */
export function compressAndEncode(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    const objectUrl = URL.createObjectURL(file);

    img.onload = () => {
      URL.revokeObjectURL(objectUrl);

      let { width, height } = img;
      if (width > MAX_DIMENSION || height > MAX_DIMENSION) {
        const scale = MAX_DIMENSION / Math.max(width, height);
        width = Math.round(width * scale);
        height = Math.round(height * scale);
      }

      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      canvas.getContext("2d")!.drawImage(img, 0, 0, width, height);

      // toDataURL returns "data:image/jpeg;base64,<data>" — strip the prefix
      const dataUrl = canvas.toDataURL("image/jpeg", JPEG_QUALITY);
      resolve(dataUrl.split(",")[1]);
    };

    img.onerror = reject;
    img.src = objectUrl;
  });
}
