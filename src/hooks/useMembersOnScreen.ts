import { onCleanup } from "solid-js";
import { governanceCommands } from "../ipc/commands/governance";

/** The most members the Host takes on screen (plan E3.4 Q1). */
const MEMBERS_ON_SCREEN_MAX = 50;
/** Scrolling settles before the list reports its rows. */
const REPORT_DEBOUNCE_MS = 500;

/**
 * A `ref` for a member list: reports the rows it shows (`[data-pseudonym]`)
 * to the Host, debounced, so their presence stays live beside the ring's
 * (plan E3.4 Q1). The Host decides what to read; the list only says what is
 * visible. On cleanup it reports none.
 */
export function useMembersOnScreen(communityId: () => string): (list: HTMLElement) => void {
  return (list) => {
    const visible = new Set<string>();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let sent = "";
    let sentFor = "";

    const send = (community: string, members: string[]) => {
      sent = members.join(",");
      sentFor = community;
      void governanceCommands.setMembersOnScreen(community, members);
    };
    const report = () => {
      timer = undefined;
      const members = [...visible].sort().slice(0, MEMBERS_ON_SCREEN_MAX);
      const community = communityId();
      if (community === sentFor && members.join(",") === sent) return;
      if (sentFor !== "" && sentFor !== community) send(sentFor, []);
      send(community, members);
    };
    const schedule = () => {
      if (timer === undefined) timer = setTimeout(report, REPORT_DEBOUNCE_MS);
    };

    const rows = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        const key = (entry.target as HTMLElement).dataset.pseudonym;
        if (!key) continue;
        if (entry.isIntersecting) visible.add(key);
        else visible.delete(key);
      }
      schedule();
    });
    const observeRows = () => {
      rows.disconnect();
      visible.clear();
      list.querySelectorAll<HTMLElement>("[data-pseudonym]").forEach((row) => rows.observe(row));
      schedule();
    };
    const changes = new MutationObserver(observeRows);
    changes.observe(list, { childList: true, subtree: true });
    observeRows();

    onCleanup(() => {
      changes.disconnect();
      rows.disconnect();
      if (timer !== undefined) clearTimeout(timer);
      if (sentFor !== "") void governanceCommands.setMembersOnScreen(sentFor, []);
    });
  };
}
