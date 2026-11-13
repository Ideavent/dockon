import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { Button } from "../../components/ui/button";
import { request, waitOperation, type Operation } from "../../ipc/client";
type Action = "hosts_update" | "hosts_remove" | "trust_ca" | "untrust_ca";
type Plan = {
  token: string;
  action: Action;
  target: string;
  before: string;
  after: string;
  fingerprint: string | null;
  warning: string;
  supported: boolean;
};
export function SystemPermissions({
  lang,
  hasCertificate,
}: {
  lang: "tr" | "en";
  hasCertificate: boolean;
}) {
  const tr = lang === "tr";
  const [plan, setPlan] = useState<Plan | null>(null);
  const [done, setDone] = useState(false);
  const preview = useMutation({
    mutationFn: (action: Action) => request<Plan>("system_plan", { action }),
    onSuccess: (p) => {
      setPlan(p);
      setDone(false);
      apply.reset();
    },
  });
  const apply = useMutation({
    mutationFn: async () => {
      const op = await request<Operation>("system_apply", {
        token: plan?.token,
      });
      await waitOperation(op.id);
    },
    onSuccess: () => {
      setDone(true);
      setPlan(null);
    },
  });
  const busy = preview.isPending || apply.isPending;
  return (
    <section className="router-plan">
      <h2>{tr ? "Sistem izinleri" : "System permissions"}</h2>
      <p className="muted">
        {tr
          ? "Hosts kayıtları ve CA güveni ayrı ayrı önizlenir ve onaylanır."
          : "Hosts records and CA trust are reviewed and approved separately."}
      </p>
      <div className="heading-actions">
        <Button disabled={busy} onClick={() => preview.mutate("hosts_update")}>
          {tr ? "Hosts kaydını önizle" : "Preview hosts entries"}
        </Button>
        <Button disabled={busy} onClick={() => preview.mutate("hosts_remove")}>
          {tr ? "Hosts kaldırmayı önizle" : "Preview hosts removal"}
        </Button>
        {hasCertificate && (
          <>
            <Button disabled={busy} onClick={() => preview.mutate("trust_ca")}>
              {tr ? "CA güvenini önizle" : "Preview CA trust"}
            </Button>
            <Button
              disabled={busy}
              onClick={() => preview.mutate("untrust_ca")}
            >
              {tr
                ? "CA güvenini kaldırmayı önizle"
                : "Preview CA trust removal"}
            </Button>
          </>
        )}
      </div>
      {[preview.error, apply.error].filter(Boolean).map((e, i) => (
        <div className="error" role="alert" key={i}>
          {String(e)}
        </div>
      ))}
      {done && (
        <div className="notice" role="status">
          {tr
            ? "İşlem tamamlandı ve sonucu doğrulandı."
            : "The operation completed and its outcome was verified."}
        </div>
      )}
      {plan && (
        <>
          <h3>{plan.target}</h3>
          <div className="notice">
            <p>
              {plan.action.startsWith("hosts")
                ? tr
                  ? "Yönetici izni istenir. Yalnızca Dockon kurulumuna ait işaretli kayıtlar değiştirilir; hosts dosyasının yanında yedek oluşturulur. Çakışan mevcut adresler değiştirilmez."
                  : "Administrator approval is required. Only this installation’s marked entries change; a backup is created next to hosts. Existing conflicting entries remain unchanged."
                : tr
                  ? "Bu CA’ya güvenmek, bu kullanıcı hesabında herhangi bir alan adı için ürettiği sertifikalara güvenmek anlamına gelir. Yalnızca kontrol ettiğin CA’ya onay ver. Bu işlem kullanıcı güven deposunu kapsar; Firefox gibi ayrı depolar ayrıca yapılandırılabilir."
                  : "Trusting this CA accepts certificates it issues for any domain in this user account. Only approve a CA you control. This changes user trust; separate stores such as Firefox may need their own configuration."}
            </p>
          </div>
          {plan.fingerprint && (
            <p>
              SHA-256: <code>{plan.fingerprint}</code>
            </p>
          )}
          <div className="config-preview">
            <div>
              <h3>{tr ? "Önce" : "Before"}</h3>
              <pre>
                {plan.before || (tr ? "Dockon kaydı yok" : "No Dockon entries")}
              </pre>
            </div>
            <div>
              <h3>{tr ? "Sonra" : "After"}</h3>
              <pre>
                {plan.after || (tr ? "Dockon kaydı yok" : "No Dockon entries")}
              </pre>
            </div>
          </div>
          {!plan.supported && (
            <div className="notice">
              {tr
                ? "Bu işletim sisteminde otomatik uygulama henüz desteklenmiyor."
                : "Automatic application is not yet supported on this operating system."}
            </div>
          )}
          <div className="heading-actions">
            <Button disabled={busy} onClick={() => setPlan(null)}>
              {tr ? "Vazgeç" : "Cancel"}
            </Button>
            <Button
              variant="primary"
              disabled={busy || !plan.supported}
              onClick={() => apply.mutate()}
            >
              {apply.isPending
                ? tr
                  ? "Sistem işlemi bekleniyor…"
                  : "Waiting for system operation…"
                : tr
                  ? "Bu sistem değişikliğine onay ver"
                  : "Approve this system change"}
            </Button>
          </div>
        </>
      )}
    </section>
  );
}
