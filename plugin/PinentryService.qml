import QtQuick
import Quickshell
import Quickshell.Io

// Answers pinentry-omarchy over $XDG_RUNTIME_DIR/pinentry-omarchy.sock:
// one NDJSON request line in, one response line out, then the plugin
// closes the connection (closing it here rather than in the client keeps
// Quickshell from logging a PeerClosedError). A client that disconnects
// (or half-closes) before the answer cancels its dialog. Only one request
// is served at a time; others get "busy".
//
// Protocol v2: the PIN goes back percent-encoded (encodeURIComponent), so
// the client parses it without JSON unescaping. A client speaking another
// version is refused rather than handed a PIN it would misread.
Item {
  id: root

  readonly property int protocolVersion: 2
  readonly property int maxRequestBytes: 65536
  property var activeConn: null

  function reply(conn, response) {
    conn.write(JSON.stringify(response) + "\n")
    conn.flush()
    conn.connected = false
  }

  function handleRequest(conn, line) {
    if (conn.handled) return
    conn.handled = true

    if (line.length > maxRequestBytes) {
      reply(conn, { result: "error", message: "request too large" })
      return
    }
    var req
    try {
      req = JSON.parse(line)
    } catch (e) {
      reply(conn, { result: "error", message: "invalid JSON" })
      return
    }
    if (!req || req.v !== protocolVersion || ["getpin", "confirm", "message"].indexOf(req.type) === -1) {
      reply(conn, { result: "error", message: "unsupported request" })
      return
    }
    if (activeConn) {
      reply(conn, { result: "busy" })
      return
    }
    activeConn = conn
    dialog.open(req)
  }

  function finish(result, pin) {
    var conn = activeConn
    activeConn = null
    if (!conn) return
    var response = { result: result }
    if (result === "ok" && dialog.mode === "getpin") {
      try {
        response.pin = encodeURIComponent(pin)
      } catch (e) {
        // Lone surrogates can't be encoded as UTF-8.
        response = { result: "error", message: "PIN is not valid Unicode" }
      }
    }
    pin = ""
    reply(conn, response)
    response = null
  }

  PinentryDialog {
    id: dialog
    onFinished: function(result, pin) { root.finish(result, pin) }
  }

  SocketServer {
    active: true
    path: Quickshell.env("PINENTRY_OMARCHY_SOCKET") || (Quickshell.env("XDG_RUNTIME_DIR") + "/pinentry-omarchy.sock")
    handler: Socket {
      id: conn
      property bool handled: false

      onConnectionStateChanged: {
        if (!connected && root.activeConn === conn) {
          root.activeConn = null
          dialog.close()
        }
      }

      parser: SplitParser {
        onRead: function(line) { root.handleRequest(conn, line) }
      }
    }
  }
}
