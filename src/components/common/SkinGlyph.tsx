import { type Component, createMemo, Show } from "solid-js";
import { skinSheetLinked } from "../../hooks/useSkin";
import { COMMUNITY_ICONS, type CommunityIcon } from "../../styles/generated/community_icon";
import { GLYPH_ART, GLYPH_EM, type GlyphName } from "./glyph_art";

/** Whether `name` is one a pack may serve (`manifest.icons`, plan SK17-5). */
function servable(name: GlyphName): name is CommunityIcon {
  return (COMMUNITY_ICONS as readonly string[]).includes(name);
}

/** The custom property a pack's icon for `name` is served as: `--skin-icon-mic-on`. */
function iconVar(name: CommunityIcon): string {
  return `--skin-icon-${name.replace(/\./g, "-")}`;
}

/**
 * An icon in a skinned window (plan SK17-7a): the pack's own image when the
 * shown skin serves one for a Community icon (`--skin-icon-<name>`), else
 * the shell's glyph, drawn as SVG in the element's colour (the role's token)
 * and the font glyph's box, never a font glyph. Decorative unless `label`
 * names it; a `title` shows on hover as before.
 */
const SkinGlyph: Component<{ name: GlyphName; class?: string; title?: string; label?: string }> = (
  props
) => {
  const served = createMemo(() => {
    skinSheetLinked();
    const name = props.name;
    if (!servable(name)) return undefined;
    const prop = iconVar(name);
    return getComputedStyle(document.documentElement).getPropertyValue(prop).trim() !== ""
      ? prop
      : undefined;
  });
  const art = () => GLYPH_ART[props.name];
  return (
    <span
      class={props.class ? `skin-glyph ${props.class}` : "skin-glyph"}
      classList={{ "skin-glyph-served": served() !== undefined }}
      data-glyph={props.name}
      style={served() ? { "--skin-glyph-image": `var(${served()})` } : undefined}
      title={props.title}
      role="img"
      aria-label={props.label}
      aria-hidden={props.label ? undefined : "true"}
    >
      <Show when={!served()}>
        <svg
          class="skin-glyph-svg"
          viewBox={`${art().x} 0 ${GLYPH_EM} ${GLYPH_EM}`}
          aria-hidden="true"
        >
          <path d={art().d} />
        </svg>
      </Show>
    </span>
  );
};

export default SkinGlyph;
