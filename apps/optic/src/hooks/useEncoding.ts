import { useCallback, useEffect, useRef, useState } from "react";
import { detectCwebp, encodeImage } from "../services/encodeService";
import type { EncodingConfig, OptimizedImage, SanitizeConfig, SourceImage } from "../types";

interface UseEncodingResult {
  optimizedImage: OptimizedImage | null;
  isEncoding: boolean;
  error: string | null;
}

export function useEncoding(
  sourceImage: SourceImage | null,
  config: EncodingConfig,
  sanitize: SanitizeConfig,
  onCwebpLost?: () => void
): UseEncodingResult {
  const [optimizedImage, setOptimizedImage] = useState<OptimizedImage | null>(
    null
  );
  const [isEncoding, setIsEncoding] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generationRef = useRef(0);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const doEncode = useCallback(
    async (
      image: SourceImage,
      cfg: EncodingConfig,
      san: SanitizeConfig,
      generation: number
    ) => {
      setIsEncoding(true);
      setError(null);
      try {
        const result = await encodeImage(image.filePath, cfg, san);
        // Only apply if this is still the latest generation
        if (generationRef.current === generation) {
          setOptimizedImage(result);
        }
      } catch (err) {
        if (generationRef.current === generation) {
          const msg = String(err);
          setError(msg);
          // Re-check binary availability on error
          try {
            const status = await detectCwebp();
            if (!status.available && onCwebpLost) {
              onCwebpLost();
            }
          } catch {
            // detection itself failed — treat as lost
            onCwebpLost?.();
          }
        }
      } finally {
        if (generationRef.current === generation) {
          setIsEncoding(false);
        }
      }
    },
    [onCwebpLost]
  );

  useEffect(() => {
    if (!sourceImage) {
      setOptimizedImage(null);
      setError(null);
      return;
    }

    generationRef.current += 1;
    const gen = generationRef.current;

    if (timerRef.current) {
      clearTimeout(timerRef.current);
    }

    timerRef.current = setTimeout(() => {
      doEncode(sourceImage, config, sanitize, gen);
    }, 400);

    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
      }
    };
  }, [sourceImage, config, sanitize, doEncode]);

  return { optimizedImage, isEncoding, error };
}
