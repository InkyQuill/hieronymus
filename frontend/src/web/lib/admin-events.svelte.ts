export function connectAdminEvents(onEvent: () => void): () => void {
  let stopped = false;
  let socket: WebSocket | undefined;
  let retry = 250;
  function connect() {
    if (stopped) return;
    const scheme = location.protocol === "https:" ? "wss:" : "ws:";
    socket = new WebSocket(`${scheme}//${location.host}/ws/admin`);
    socket.onopen = () => {
      retry = 250;
      onEvent();
    };
    socket.onmessage = () => onEvent();
    socket.onclose = () => {
      if (!stopped) setTimeout(connect, retry);
      retry = Math.min(retry * 2, 5_000);
    };
  }
  connect();
  return () => {
    stopped = true;
    socket?.close();
  };
}
