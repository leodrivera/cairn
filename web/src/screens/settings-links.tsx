// Share links: the list of active links. Creating and listing them comes with the sharing work.
import { t } from "../i18n/index";
import { StatePanel } from "../components/states";

export function Links() {
  return (
    <section class="settings-page">
      <header class="settings-head">
        <h1>{t("settings.links")}</h1>
        <p class="muted">{t("links.lead")}</p>
      </header>
      <StatePanel icon="link" title={t("links.empty")} body={t("links.emptyBody")} />
    </section>
  );
}
