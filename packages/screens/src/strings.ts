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
    collections: "Collections",
    sharing: "Sharing",
    activity: "Activity",
    settings: "Settings",
  },

  common: {
    cancel: "Cancel",
    close: "Close",
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
    title: "This page doesn't exist",
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
    emptyTitle: "No collections yet",
    emptyBody: "A collection holds one kind of data, such as notes or photos. Create your first one.",
    notShared: "Only you",
    sharedWith: (names: string[]) => `Shared with ${names.join(", ")}`,
    fromElsewhere: "From another data center",
    storedHere: "Stored on this computer",
    share: "Share…",
    remove: "Remove Collection",
    removeConfirm: (name: string) =>
      `Remove ${name}? Sharing it stops. Its files stay in your data folder.`,
    removeYes: "Remove",
    whoCanSee: "Who can see this",
    manage: "Manage",
    nothingYetTitle: (name: string) => `Nothing in ${name} yet`,
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
    canView: (name: string) => `${name} can view live`,
    canCopy: (name: string) => `${name} can keep a copy`,
    until: (date: string) => `Until ${date}`,
    stopped: "Stopped",
    expiredOn: (date: string) => `Expired ${date}`,
    collectionLabel: "Collection",
    withLabel: "With",
    howLabel: "How they see it",
    viewLive: "View live",
    viewLiveBody: "Stays on your computer. Unavailable while it's off.",
    keepCopy: "Keep a copy",
    keepCopyBody: "They keep a copy. Stopping later ends updates, but the copy stays with them.",
    keepCopyUnavailable: "This collection can only be viewed live.",
    endsLabel: "Ends",
    ends7: "In 7 days",
    ends30: "In 30 days",
    endsYear: "In a year",
    share: "Share",
    needContact: "Add a contact first: sharing is with people you know.",
    needCollection: "Create a collection first.",
    inviteTitle: (name: string) => `Send this invite code to ${name}`,
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
    removeConfirm: (name: string) => `Remove ${name}? Everything you share with them stops.`,
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
