import { Component } from "solid-js";
import { ICON_CHEVRON_DOWN, ICON_CHEVRON_RIGHT } from "../../icons";

interface CategoryHeaderProps {
  name: string;
  isExpanded: boolean;
  onToggle: () => void;
}

const CategoryHeader: Component<CategoryHeaderProps> = (props) => {
  return (
    <div class="category-header" onClick={props.onToggle}>
      <span class="nf-icon category-chevron">
        {props.isExpanded ? ICON_CHEVRON_DOWN : ICON_CHEVRON_RIGHT}
      </span>
      <span class="category-name">{props.name}</span>
    </div>
  );
};

export default CategoryHeader;
