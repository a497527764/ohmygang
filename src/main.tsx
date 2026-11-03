import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

// 关掉浏览器自带的右键菜单（返回、刷新、检查之类，对聊天软件没有意义）；
// 输入框里保留，方便剪切、复制、粘贴。群列表和消息上的右键由各自的组件弹出应用自己的菜单。
document.addEventListener("contextmenu", (e) => {
  if (!(e.target as HTMLElement | null)?.closest("input, textarea")) e.preventDefault();
});

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
