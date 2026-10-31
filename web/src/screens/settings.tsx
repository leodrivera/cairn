// The settings frame: the sidebar lists the settings pages instead of the places. Users and Server are for admins.
// Share links are a place of their own, beside My files, not a settings page.
import { t } from "../i18n/index";
import type { SettingsPage } from "../lib/router";
import { Places, Shell } from "../components/shell";
import { useUser } from "../state/session";
import { Profile } from "./settings-profile";
import { Users } from "./settings-users";
import { ServerSettings } from "./settings-server";

export function Settings({ page }: { page: SettingsPage }) {
  const user = useUser();
  const admin = user.permissions.admin;
  const shown: SettingsPage = !admin && (page === "users" || page === "server") ? "profile" : page;
  const items = [
    { href: "/files/", label: t("settings.back"), icon: "back" as const, on: false },
    { href: "/settings/profile", label: t("settings.profile"), icon: "user" as const, on: shown === "profile" },
    ...(admin ? [{ href: "/settings/users", label: t("settings.users"), icon: "users" as const, on: shown === "users" }] : []),
    ...(admin
      ? [{ href: "/settings/server", label: t("settings.server"), icon: "sliders" as const, on: shown === "server" }]
      : []),
  ];
  return (
    <Shell class="settings" nav={<Places items={items} class="places settings-nav" />}>
      {shown === "profile" && <Profile />}
      {shown === "users" && <Users />}
      {shown === "server" && <ServerSettings />}
    </Shell>
  );
}
