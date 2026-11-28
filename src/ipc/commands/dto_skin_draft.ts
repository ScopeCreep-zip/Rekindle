// The skin editor's wire types (plan E7.7.5; `rekindle_types::dto::{skin,
// skin_draft}`): the draft the editor holds, the Host's report on it and
// the token, region and licence shapes both carry. Split from `./dto`
// along the editor's seam; the library and applied-skin shapes stay there.

import type { SkinCredits } from "./dto";

/// A design-token value (`rekindle_types::dto::skin::TokenValue`).
export type TokenValue =
  | { type: "color"; data: { r: number; g: number; b: number; a: number } }
  | { type: "dimension"; data: number }
  | { type: "number"; data: number }
  | { type: "fontFamily"; data: string };

/// How a region image fills its box (`rekindle_types::dto::skin::RegionFit`).
export type RegionFit =
  | { type: "stretch" }
  | { type: "tile" }
  | { type: "nineSlice"; data: { top: number; right: number; bottom: number; left: number } };

/// A skin's licence (`SkinLicense`): an SPDX expression or its full text.
type SkinLicense = { spdx: string } | { text: string };

/// One imported region (`SkinDraftRegion`): an image the Host reads and
/// the validator checks.
interface SkinDraftRegion {
  path: string;
  fit: RegionFit;
  textBacking: string | null;
}

/// A registry region an image may be imported for (`SkinRegionSlot`).
interface SkinRegionSlot {
  id: string;
  carriesText: boolean;
}

/// A skin being edited (`rekindle_types::dto::skin_draft::SkinDraft`, plan
/// E7.7.5): one variant's tokens by registry id, its author, licence and
/// the credits it carries from its source, and its imported regions.
export interface SkinDraft {
  name: string;
  description: string;
  author: string;
  /// An SPDX expression or the licence's full text (the Host reads which).
  license: string;
  credits: SkinCredits;
  base: "light" | "dark";
  contrast: "standard" | "high";
  tokens: Record<string, TokenValue>;
  /// The package it started from (`builtin:<id>` or `pkg:<hash>`).
  from: string | null;
  regions: Record<string, SkinDraftRegion>;
}

/// One themeable registry key, resolved (`SkinDraftToken`).
interface SkinDraftToken {
  id: string;
  css: string;
  value: TokenValue;
  set: boolean;
}

/// A passing colour for a failing pair (`SkinSuggestion`).
interface SkinSuggestion {
  token: string;
  rgb: [number, number, number];
  ratioX100: number;
}

/// One declared contrast pair, measured by the Host (`SkinPairCheck`).
export interface SkinPairCheck {
  rule: string;
  fg: string;
  ground: string;
  ratioX100: number;
  requiredX100: number;
  pass: boolean;
  apcaLcX10: number;
  suggestion: SkinSuggestion | null;
}

/// The Host's report on a draft (`SkinDraftReport`).
export interface SkinDraftReport {
  tokens: SkinDraftToken[];
  pairs: SkinPairCheck[];
  failures: string[];
  tokensCss: string;
  authorRequired: string | null;
  exportable: boolean;
  /// The licence and credits the export carries, as the Host reads them.
  license: SkinLicense;
  credits: SkinCredits;
  regionSlots: SkinRegionSlot[];
}
