import heartsteel from "./assets/heartsteel.png";

// 占位页：正式界面在 M1-5 实现，这里只确认应用能启动、图片能加载。
function App() {
  return (
    <main
      style={{
        minHeight: "100vh",
        display: "flex",
        flexDirection: "column",
        alignItems: "center",
        justifyContent: "center",
        gap: 12,
      }}
    >
      <img src={heartsteel} alt="心之钢" width={160} height={160} style={{ borderRadius: 24 }} />
      <h1 style={{ margin: 0 }}>心之钢群聊</h1>
      <p style={{ margin: 0, opacity: 0.6 }}>界面开发中</p>
    </main>
  );
}

export default App;
