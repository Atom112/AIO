import { Component } from 'solid-js';

interface IconProps {
  src: string;
  class?: string;
}

/**
 * 通用图标组件
 * 使用 CSS mask-image 加载外部 SVG 文件，保持 currentColor 颜色继承
 */
const Icon: Component<IconProps> = (props) => {
  return (
    <span
      class={`block bg-current ${props.class || ''}`}
      style={{
        "-webkit-mask-image": `url(${props.src})`,
        "mask-image": `url(${props.src})`,
        "-webkit-mask-size": "contain",
        "mask-size": "contain",
        "-webkit-mask-repeat": "no-repeat",
        "mask-repeat": "no-repeat",
        "-webkit-mask-position": "center",
        "mask-position": "center",
      }}
    />
  );
};

export default Icon;
