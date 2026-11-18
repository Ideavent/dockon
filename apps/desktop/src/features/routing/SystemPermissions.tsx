import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button } from "../../components/ui/button";
import { request, waitOperation, type Operation } from "../../ipc/client";

export function SystemPermissions({
  lang,
  hostsPending,
  trustPending = false,
}: {
  lang: "tr" | "en";
  hostsPending: boolean;
  trustPending?: boolean;
}) {
  const tr = lang === "tr";
  const cache = useQueryClient();
  const apply = useMutation({
    mutationFn: async (action: "hosts_update" | "trust_ca") => {
      const plan = await request<{
        token: string;
        before: string;
        after: string;
      }>("system_plan", { action });
      if (action === "hosts_update" && plan.before === plan.after) return;
      const operation = await request<Operation>("system_apply", {
        token: plan.token,
      });
      await waitOperation(operation.id);
    },
    onSettled: () => cache.invalidateQueries({ queryKey: ["router"] }),
  });
  if (!hostsPending && !trustPending && !apply.error) return null;
  return (
    <div className="heading-actions">
      {hostsPending && (
        <Button
          disabled={apply.isPending}
          onClick={() => apply.mutate("hosts_update")}
        >
          {tr ? "Yerel adresleri etkinleştir" : "Enable local domains"}
        </Button>
      )}
      {trustPending && (
        <Button
          variant="secondary"
          disabled={apply.isPending}
          onClick={() => apply.mutate("trust_ca")}
        >
          {tr
            ? "Dockon SSL sertifikasına güven"
            : "Trust Dockon's SSL certificate"}
        </Button>
      )}
      {apply.isPending && (
        <span role="status">
          {tr ? "Sistem onayı bekleniyor…" : "Waiting for system approval…"}
        </span>
      )}
      {apply.error && <p role="alert">{apply.error.message}</p>}
    </div>
  );
}
