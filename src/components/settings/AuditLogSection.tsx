import { Component, Show, createSignal } from "solid-js";
import { commands } from "../../ipc/commands";
import type { AuditExported, AuditVerified } from "../../ipc/commands/dto";
import { errorMessage } from "../../utils/error";

/// The identity's tamper-evident audit log (plan E7.15 S3 item 3): the
/// Host verifies the whole chain (`AuditVerify`, a read, no review) and a
/// broken chain also raises the Host's tamper alert. Export asks the shell
/// for a place to save and the Host writes the whole log there
/// (`AuditExport`, Protected: the intent dialog reviews it first). This
/// section only shows the Host's answers.
const AuditLogSection: Component = () => {
  const [result, setResult] = createSignal<AuditVerified | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [exported, setExported] = createSignal<AuditExported | null>(null);
  const [busy, setBusy] = createSignal(false);

  const verify = async () => {
    setBusy(true);
    setError(null);
    try {
      setResult(await commands.auditVerify());
    } catch (e) {
      setResult(null);
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const exportLog = async () => {
    setBusy(true);
    setError(null);
    try {
      const written = await commands.auditExport(0);
      if (written !== null) setExported(written);
    } catch (e) {
      setExported(null);
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const summary = (r: AuditVerified): string =>
    r.state === "intact"
      ? `The audit log is intact (${r.length} entries).`
      : `The audit log is broken${r.at === null ? "" : ` at entry ${r.at}`} of ${r.length}: ${r.detail}`;

  return (
    <>
      <div class="settings-section-title">Audit Log</div>
      <div class="settings-hint">
        Every security-relevant action on this identity is chained in a tamper-evident log. Verify checks
        the whole chain; Export saves it to a file you choose.
      </div>
      <Show when={error()}>
        <div class="form-error" role="alert">{error()}</div>
      </Show>
      <Show when={result()}>
        {(r) => (
          <div class={r().state === "intact" ? "settings-hint" : "form-error"} role="status">
            {summary(r())}
          </div>
        )}
      </Show>
      <Show when={exported()}>
        {(x) => (
          <div class="settings-hint" role="status">
            Exported {x().entries} entries to {x().path}.
          </div>
        )}
      </Show>
      <div class="form-field-row">
        <button class="form-btn-secondary" disabled={busy()} onClick={() => void verify()}>
          Verify audit log
        </button>
        <button class="form-btn-secondary" disabled={busy()} onClick={() => void exportLog()}>
          Export audit log…
        </button>
      </div>
    </>
  );
};

export default AuditLogSection;
