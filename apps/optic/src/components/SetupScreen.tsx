import { useCallback, useState } from "react";
import type { CwebpStatus } from "../types";
import { useTranslation } from "../i18n";
import { detectCwebp } from "../services/encodeService";
import "./SetupScreen.css";

interface SetupScreenProps {
  onCwebpDetected: (status: CwebpStatus) => void;
  /** Documents mode works without cwebp. */
  onContinueWithDocuments?: () => void;
}

export default function SetupScreen({ onCwebpDetected, onContinueWithDocuments }: SetupScreenProps) {
  const [checking, setChecking] = useState(false);
  const [lastCheck, setLastCheck] = useState<CwebpStatus | null>(null);

  const handleCheckAgain = useCallback(async () => {
    setChecking(true);
    try {
      const status = await detectCwebp();
      setLastCheck(status);
      if (status.available) {
        onCwebpDetected(status);
      }
    } catch {
      setLastCheck({ available: false, binaryPath: null, version: null, platform: "unknown" });
    } finally {
      setChecking(false);
    }
  }, [onCwebpDetected]);

  const { t } = useTranslation();
  const platform = lastCheck?.platform ?? "unknown";

  return (
    <div className="setup-screen">
      <div className="setup-screen__card">
        <div className="setup-screen__icon">⚙️</div>
        <h1 className="setup-screen__title">{t("setup.title")}</h1>
        <p className="setup-screen__description">
          {t("setup.description")}
        </p>

        <div className="setup-screen__instructions">
          <InstructionBlock
            title={t("setup.windows")}
            highlight={platform === "windows"}
            steps={[
              t("setup.windowsStep1"),
              t("setup.windowsStep2"),
              t("setup.windowsStep3"),
            ]}
            command={null}
          />
          <InstructionBlock
            title={t("setup.macos")}
            highlight={platform === "macos"}
            steps={[t("setup.macosStep")]}
            command="brew install webp"
          />
          <InstructionBlock
            title={t("setup.linuxDebian")}
            highlight={platform === "linux"}
            steps={[t("setup.debianStep")]}
            command="sudo apt install webp"
          />
          <InstructionBlock
            title={t("setup.linuxFedora")}
            highlight={platform === "linux"}
            steps={[t("setup.fedoraStep")]}
            command="sudo dnf install libwebp-tools"
          />
        </div>

        {lastCheck && !lastCheck.available && (
          <p className="setup-screen__error">
            {t("setup.notFound")}
          </p>
        )}

        <button
          className="setup-screen__button"
          onClick={handleCheckAgain}
          disabled={checking}
        >
          {checking ? t("setup.checking") : t("setup.checkAgain")}
        </button>
        {onContinueWithDocuments && (
          <button className="setup-screen__button setup-screen__button--secondary" onClick={onContinueWithDocuments}>
            {t("setup.continueDocs")}
          </button>
        )}
      </div>
    </div>
  );
}

function InstructionBlock({
  title,
  highlight,
  steps,
  command,
}: {
  title: string;
  highlight: boolean;
  steps: string[];
  command: string | null;
}) {
  return (
    <div className={`instruction-block ${highlight ? "instruction-block--highlight" : ""}`}>
      <h3 className="instruction-block__title">{title}</h3>
      {steps.map((step, i) => (
        <p key={i} className="instruction-block__step">{step}</p>
      ))}
      {command && (
        <pre className="instruction-block__command">{command}</pre>
      )}
    </div>
  );
}
