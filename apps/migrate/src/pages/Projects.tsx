import { useEffect, useState } from "react";
import { useI18n } from "../i18n";
import { api } from "../lib/api";
import type { SpiderProject } from "../lib/api";
import { useProjects, useToast } from "../lib/context";
import { formatDate } from "../lib/format";
import { Card, DataTable, Empty, ErrorAlert, Field, Page } from "../components/ui";

export default function Projects() {
  const { t, locale } = useI18n();
  const toast = useToast();
  const { projects, project, setProjectId, reload, error } = useProjects();
  const [name, setName] = useState("");
  const [domain, setDomain] = useState("");
  const [spider, setSpider] = useState("");
  const [spiderProjects, setSpiderProjects] = useState<SpiderProject[]>([]);
  const [busy, setBusy] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    api.spiderProjects().then((r) => setSpiderProjects(r.projects)).catch(() => setSpiderProjects([]));
  }, []);

  const create = async () => {
    setBusy(true);
    setFormError(null);
    try {
      const p = await api.createProject(name, domain, spider);
      await reload();
      setProjectId(p.id);
      setName("");
      setDomain("");
      setSpider("");
      toast(t("project.created", { name: p.name }), "success");
    } catch (e) {
      setFormError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Page title={t("nav.projects")} subtitle={t("project.emptyText")}>
      <ErrorAlert error={error} />
      <Card title={t("project.new")}>
        <div className="grid grid--3">
          <Field label={t("project.name")}>
            <input className="input" value={name} placeholder={t("project.namePlaceholder")} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && name.trim() && create()} />
          </Field>
          <Field label={t("project.domain")}>
            <input className="input" value={domain} placeholder={t("project.domainPlaceholder")} onChange={(e) => setDomain(e.target.value)} />
          </Field>
          <Field label={t("project.spider")} hint={t("project.spiderHint")}>
            <select className="select" value={spider} onChange={(e) => setSpider(e.target.value)}>
              <option value="">—</option>
              {spiderProjects.map((p) => <option key={p.name} value={p.name}>{p.name}</option>)}
            </select>
          </Field>
        </div>
        <ErrorAlert error={formError} />
        <div className="row">
          <button type="button" className="btn btn--primary" disabled={busy || !name.trim()} onClick={create}>{t("common.create")}</button>
        </div>
      </Card>
      {projects.length === 0 ? (
        <Empty title={t("project.emptyTitle")} text={t("project.emptyText")} />
      ) : (
        <Card>
          <DataTable
            rows={projects}
            rowKey={(p) => p.id}
            onRowClick={(p) => setProjectId(p.id)}
            columns={[
              { key: "name", label: t("project.name"), render: (p) => <b style={{ color: p.id === project?.id ? "var(--accent-hover)" : undefined }}>{p.name}</b> },
              { key: "domain", label: t("project.domain") },
              { key: "spiderProject", label: t("project.spider") },
              { key: "createdAt", label: "", render: (p) => <span className="muted">{formatDate(p.createdAt, locale, false)}</span> },
              {
                key: "actions", label: "", sortable: false, render: (p) => (
                  <button type="button" className="btn btn--danger btn--sm" onClick={async (e) => {
                    e.stopPropagation();
                    if (!window.confirm(t("common.confirmDelete", { name: p.name }))) return;
                    try {
                      await api.deleteProject(p.id);
                      await reload();
                      toast(t("project.deleted"), "success");
                    } catch (err) {
                      toast(String(err instanceof Error ? err.message : err), "error");
                    }
                  }}>{t("common.delete")}</button>
                ),
              },
            ]}
          />
        </Card>
      )}
    </Page>
  );
}
