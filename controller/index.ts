console.log("Starting server");

Bun.serve({
  fetch(req, res) {
    console.log("Request received");
    if (res.upgrade(req)) {
      return;
    } else {
      return new Response("Internal Server Error", {
        status: 500,
      });
    }
  },
  websocket: {
    message(ws, message) {
      
    },
    open(ws) {},
    close(ws, code, reason) {},
    drain(ws) {},
  },
  port: 1091,
  reusePort: true,
});
