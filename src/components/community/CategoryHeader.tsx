import type { Component } from "solid-js";
import SkinGlyph from "../common/SkinGlyph";

interface CategoryHeaderProps {
  name: string;
  isExpanded: boolean;
  onToggle: () => void;
}

const CategoryHeader: Component<CategoryHeaderProps> = (props) => {
  return (
    <button
      type="button"
      class="category-header"
      aria-expanded={props.isExpanded}
      onClick={props.onToggle}
    >
      {/* The category's fold arrow (plan SK17-7a: fold.open / fold.closed). */}
      <SkinGlyph name={props.isExpanded ? "fold.open" : "fold.closed"} class="category-chevron" />
      <span class="category-name">{props.name}</span>
    </button>
  );
};

export default CategoryHeader;
