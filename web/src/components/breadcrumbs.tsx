// Where the open folder is: "My files" (or the granted folder's name) then each folder, readable even for names that
// are not UTF-8.
import { Link } from "../lib/router";
import { displaySegment, segments } from "../lib/paths";
import { useWhere } from "../lib/where";
import { t } from "../i18n/index";

export function Breadcrumbs({ path }: { path: string }) {
  const where = useWhere();
  const segs = segments(path);
  const crumbs = [{ label: where.grant?.name ?? t("nav.myFiles"), path: "" }].concat(
    segs.map((s, i) => ({ label: displaySegment(s), path: segs.slice(0, i + 1).join("/") })),
  );
  return (
    <nav class="breadcrumbs" aria-label={t("explorer.breadcrumbs")}>
      {crumbs.map((c, i) => {
        const last = i === crumbs.length - 1;
        return (
          <>
            {i > 0 && <span aria-hidden="true">/</span>}
            {last ? <span aria-current="page">{c.label}</span> : <Link href={where.files(c.path)}>{c.label}</Link>}
          </>
        );
      })}
    </nav>
  );
}
