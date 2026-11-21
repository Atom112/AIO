/**
 * Team settings page: the team cloud account card (server link, device sign-in,
 * workspace binding, pending invitations and sync controls) on its own page.
 *
 * Split out of the app-settings page so team identity and sync live next to the
 * other team-facing settings instead of below the app version card.
 */
import { Component } from 'solid-js';
import { CloudAccountCard } from './components/CloudAccountCard';

const TeamSettingsPage: Component = () => (
  <div class="flex flex-col gap-[15px] box-border">
    <CloudAccountCard />
  </div>
);

export default TeamSettingsPage;
