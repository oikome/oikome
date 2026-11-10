// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// British English: American English with British spelling and words. Dates
// and numbers already come out the British way (`format.ts`).

import { en } from "./en";
import type { Strings } from "./en";
import { overlay } from "./helpers";

export const enGB: Strings = overlay(en, {
  brandSubtitle: "My data centre",
  defaultName: "My data centre",
  toast: {
    renamed: (name: string) => `Your data centre is now called ${name}.`,
  },
  reasons: {
    invite_other: "This invite is for another data centre; ask them to share with your ID, from Settings.",
  },
  backup: {
    bannerBody:
      "It is your identity. If you lose this computer, no one else can recover your data centre.",
  },
  recovery: {
    lede:
      "This key is your identity. If this computer is lost, the key is the only way to get your data centre back. No one else has a copy.",
    fileIntro: "Oikome recovery key. Keep it secret: whoever has it can act as your data centre.",
  },
  onboarding: {
    welcomeLede: "Your own data centre, on your own computer. Two choices, both already made for you.",
    namedAfter: (first: string) => `${first}'s data centre`,
    restoreHelp:
      "To restore a data centre you had before, stop Oikome, run `oikome identity restore` with a new, empty data folder, and start Oikome again.",
    doneLede:
      "Your data centre is ready, with an example to look around: one weekend in Lisbon, kept as photos, notes and spending. Remove it whenever you like.",
  },
  home: {
    center: "This data centre",
    keySaved: "Your recovery key is stored. With it, this data centre can come back on a new computer.",
  },
  collections: {
    fromElsewhere: "From another data centre",
    youGetLive: "View live: read from their data centre",
  },
  sharing: {
    inviteBody: (name: string, space: string) =>
      `Send it to ${name} any way you like. When they add it in Oikome, ${space} shows up under Shared With You. Only ${name}'s data centre can use it, and it is shown only now.`,
    acceptFailed: (from: string) => `Couldn't add it. Check that ${from}'s data centre is on, then try again.`,
  },
  activity: {
    setUp: (who: string) => `${who} set up this data centre`,
    detailSetUp:
      "You set up this data centre on this computer. What it keeps is in your data folder, as plain files and its own records.",
    detailKey:
      "You said the recovery key is stored somewhere safe. With it, this data centre can be brought back on a new computer.",
    detailCopy: (who: string, what: string) => `${who}'s data centre fetched the newest copy of ${what}.`,
  },
  settings: {
    recoveryBody:
      "It is your identity. With it, this data centre can be brought back on a new computer.",
    centerTitle: "This Data Centre",
  },
});
