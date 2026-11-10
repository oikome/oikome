// Copyright 2025 The Oikome Authors
// SPDX-License-Identifier: Apache-2.0
//
// The Oikome screens: render <App client={...} /> and import
// "@oikome/screens/screens.css" (with the design system's CSS).

export { App } from "./App";
export { strings } from "./strings";
export type { Strings } from "./strings";
export { LANGUAGES, language, setLanguage, startLanguage } from "./i18n/languages";
export type { Language } from "./i18n/languages";
export { applyTheme, savedTheme } from "./theme";
export type { Theme } from "./theme";
