// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// Every string the screens show, in English. Other languages arrive through
// i18n later; keep sentences whole so they can be translated as units.
// Words for people, not engines: Collection (not Space), Contact, Share,
// Activity, Your ID (not DID).

export const strings = {
  brand: "Oikome",
  brandSubtitle: "My data center",
  defaultName: "My data center",

  nav: {
    label: "Main",
    running: "Running on this computer",
    collections: "Collections",
    sharing: "Sharing",
    activity: "Activity",
    settings: "Settings",
  },

  common: {
    cancel: "Cancel",
    close: "Close",
    skip: "Skip to Content",
    nameRequired: "Enter a name.",
    moreActions: (name: string) => `More actions for ${name}`,
    save: "Save",
    done: "Done",
    copy: "Copy",
    copied: "Copied",
    loading: "Loading…",
    retry: "Try Again",
    you: "You",
    unknown: "Someone unknown",
  },

  /** One short line after an action went through. */
  toast: {
    created: (name: string) => `Created ${name}.`,
    removed: (name: string) => `Removed ${name}.`,
    stopped: (space: string, name: string) => `Stopped sharing ${space} with ${name}.`,
    contactAdded: (name: string) => `Added ${name} to your contacts.`,
    contactRenamed: (name: string) => `Renamed to ${name}.`,
    renamed: (name: string) => `Your data center is now called ${name}.`,
    keySaved: "Recovery key backed up.",
  },

  errors: {
    network: "Oikome is not answering. Check that it is still running, then try again.",
    session: "You were signed out. Run `oikome web open` on this computer to sign in again.",
    generic: "Something went wrong. Try again in a moment.",
    refused: "You don't have access to this.",
    gone: "This isn't here any more. Reload the page to see what's current.",
    conflict: "That already exists.",
    invalid: "Oikome couldn't do that. Check what you entered, then try again.",
  },

  notFound: {
    title: "This Page Doesn't Exist",
  },

  backup: {
    bannerTitle: "Back up your recovery key",
    bannerBody:
      "It is your identity. If you lose this computer, no one else can recover your data center.",
    backUpNow: "Back Up Now",
  },

  recovery: {
    title: "Save Your Recovery Key",
    lede:
      "This key is your identity. If this computer is lost, the key is the only way to get your data center back. No one else has a copy.",
    hint: "Store it on a USB drive or in a password manager.",
    download: "Download Key File",
    stored: "I've stored it somewhere safe",
    later: "Remind Me Later",
    fileName: "oikome-recovery.key",
    fileIntro: "Oikome recovery key. Keep it secret: whoever has it can act as your data center.",
  },

  onboarding: {
    stepOf: (step: number, total: number) => `Step ${step} of ${total}`,
    welcomeTitle: "Welcome to Oikome",
    welcomeLede: "Your own data center, running on your own computer. Give it a name to start.",
    nameLabel: "Name",
    keptHere: "Stored on this computer",
    restoreLink: "Restore from a recovery key",
    restoreHelp:
      "To restore a data center you had before, stop Oikome, run `oikome identity restore` with a new, empty data folder, and start Oikome again.",
    continue: "Continue",
    doneTitle: "You're All Set",
    doneLede:
      "Your data center is ready. Create a collection, then share it with people you trust. Everything stays on this computer.",
    goToCollections: "Go to Collections",
  },

  collections: {
    title: "Collections",
    newCollection: "New Collection",
    mine: "My Collections",
    sharedWithYou: "Shared With You",
    nameLabel: "Name",
    nameHint: "Letters, numbers, - and _, starting with a letter. You can't rename it later.",
    nameInvalid:
      "Use only letters, numbers, - and _, start with a letter or _, and keep it to 64 characters.",
    nameTaken: (name: string) => `You already have a collection named ${name}.`,
    cantRead:
      "This collection can't be read right now. Whoever shared it may have stopped sharing, or their computer may be off.",
    create: "Create",
    emptyTitle: "No Collections Yet",
    emptyBody: "A collection holds one kind of data, such as notes or photos. Create your first one.",
    notShared: "Only you",
    sharedWith: (names: string[]) => `Shared with ${names.join(", ")}`,
    fromElsewhere: "From another data center",
    storedHere: "Stored on this computer",
    share: "Share…",
    remove: "Remove Collection",
    removeTitle: (name: string) => `Remove ${name}?`,
    removeBody: "Sharing it stops. Its files stay in your data folder.",
    removeYes: "Remove",
    whoCanSee: "Who can see this",
    /** "Only you can see this." / "You and Sam can see this." / "You, Sam and Alex can see this." */
    whoSentence: (names: string[]) =>
      names.length === 0
        ? "Only you can see this."
        : names.length === 1
          ? `You and ${names[0]} can see this.`
          : `You, ${names.slice(0, -1).join(", ")} and ${names[names.length - 1]} can see this.`,
    manage: "Manage",
    nothingYetTitle: "No Data Yet",
    nothingYetBody:
      "Sources that fill collections arrive in a later version. Until then, you can add data in Settings, under Advanced.",
    openAdvanced: "Open Advanced",
    tableLabel: "Table",
    rows: (from: number, to: number) => `Rows ${from}–${to}`,
    noRows: "No rows",
    previous: "Previous",
    next: "Next",
    pick: "Choose a collection to see what's in it.",
  },

  sharing: {
    title: "Sharing",
    newShare: "New Share",
    sharedByYou: "Shared by You",
    nothingShared: "You haven't shared anything yet.",
    ended: "Ended",
    stop: "Stop Sharing",
    /** A share as a sentence, on the Sharing page: "Sam can view live until Dec 2, 2026." */
    shareSentence: (name: string, copy: boolean, date: string) =>
      copy ? `${name} keeps a copy until ${date}.` : `${name} can view live until ${date}.`,
    /** The same share inside the dialog, under the person's name. */
    accessUntil: (copy: boolean, date: string) =>
      copy ? `Keeps a copy until ${date}` : `Can view live until ${date}`,
    stoppedFor: (name: string) => `Stopped for ${name}.`,
    expiredFor: (name: string, date: string) => `Ended for ${name} on ${date}.`,
    shareTitle: (name: string) => `Share \u201c${name}\u201d`,
    collectionLabel: "Collection",
    withLabel: "With",
    accessLabel: "Access",
    accessView: "Can view live",
    accessCopy: "Can keep a copy",
    accessCopyUnavailable: "Can keep a copy (not for this collection)",
    viewHelp: "They read it from this computer while it's on. Nothing is copied to them.",
    copyHelp: "They keep a copy that stays up to date. If you stop sharing, their copy stays with them.",
    peopleWithAccess: "People with access",
    owner: "Owner",
    endsLabel: "Ends",
    ends7: "In 7 days",
    ends30: "In 30 days",
    endsYear: "In a year",
    share: "Share",
    needContact: "Add a contact first: sharing is with people you know.",
    needCollection: "Create a collection first.",
    inviteTitle: (name: string) => `Send ${name} This Invite Code`,
    inviteBody: (name: string) =>
      `Only ${name}'s data center can use it, until the share ends or you stop it. It is shown only now.`,
    contacts: "Contacts",
    addContact: "Add Contact",
    contactName: "Name",
    contactId: "Their ID",
    contactIdHint: "They find it in Oikome, under Settings.",
    idInvalid: "That isn't an Oikome ID. Ask them to copy theirs from Settings.",
    idIsYours: "That's your own ID. Add the ID of the person you want to share with.",
    idTaken: (name: string) => `${name} already has this ID.`,
    add: "Add",
    rename: "Rename",
    remove: "Remove",
    removeTitle: (name: string) => `Remove ${name}?`,
    removeBody: "Everything you share with them stops.",
    idRequired: "Paste their ID.",
    sharesCount: (n: number) => (n === 1 ? "You share 1 collection" : `You share ${n} collections`),
    noShares: "Nothing shared",
    contactsFootnote:
      "Contact names are stored only on this computer. The other person never sees them.",
    yourId: "Your ID",
    yourIdBody: "Give it to people who want to share with you.",
  },

  activity: {
    title: "Activity",
    recent: "Recent",
    filterLabel: "Show",
    everyone: "Everyone",
    notAContact: "Someone not in your contacts",
    allowed: "Allowed",
    refused: "Refused",
    empty: "No one has asked for your data yet.",
    emptyTitle: "No Activity Yet",
    emptyBody:
      "When someone reads a collection you share, or is turned away, it shows here. Activity is kept only on this computer, for 90 days.",
    footnote: "Activity is kept only on this computer, for 90 days.",
    more: "Show More",
    read: (who: string, what: string) => `${who} read ${what}`,
    browsed: (who: string, what: string) => `${who} looked at ${what}`,
    copied: (who: string, what: string) => `${who} copied ${what}`,
    turnedAway: (who: string, what: string) => `${who} was turned away from ${what}`,
    failed: (who: string, what: string) => `${who} couldn't read ${what}`,
    times: (n: number) => `${n} times`,
    yourData: "your data",
  },

  settings: {
    title: "Settings",
    recoveryTitle: "Recovery Key",
    backedUp: "Backed up",
    notBackedUp: "Not backed up yet",
    recoveryBody:
      "Your recovery key is your identity. If this computer is lost, it's the only way to get your data center back, and no one else has a copy.",
    showAgain: "Show Again",
    centerTitle: "This Data Center",
    nameLabel: "Name",
    rename: "Rename",
    idFootnote: "Give your ID to people who want to share with you.",
    version: "Version",
    dataFolder: "Data folder",
    preferencesTitle: "Preferences",
    languageTitle: "Language",
    language: "English",
    languageMore: "More languages are on the way.",
    advancedTitle: "Advanced",
    sqlTitle: "SQL Workbench",
    sqlBody: "For people who know SQL. Changes take effect immediately.",
  },

  sql: {
    back: "Settings",
    title: "SQL Workbench",
    lede:
      "Run statements on your data center as its owner. Separate statements with semicolons.",
    label: "Statements",
    run: "Run",
    done: (n: number | null) =>
      n === null ? "Done." : n === 1 ? "Done. 1 row changed." : `Done. ${n} rows changed.`,
    truncated: "Only the first rows are shown.",
    rowCount: (n: number) => (n === 1 ? "1 row" : `${n} rows`),
  },
};
