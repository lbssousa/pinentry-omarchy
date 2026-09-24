import QtQuick
import Quickshell
import Quickshell.Wayland
import qs.Commons
import qs.Ui

// Overlay dialog modelled on the shell's polkit agent
// (/usr/share/omarchy/shell/plugins/polkit/PolkitAgent.qml): same layer,
// scrim, card surface, [polkit] colours and failure shake.
Item {
  id: root

  signal finished(string result, string pin)

  property string mode: "getpin"
  property string title: ""
  property string desc: ""
  property string prompt: ""
  property string error: ""
  property string okLabel: ""
  property string cancelLabel: ""
  property string notokLabel: ""
  property string repeatPrompt: ""
  property string repeatError: ""
  property int timeout: 0

  property bool shown: false
  property bool closing: false
  property bool errorFlash: false
  property string localError: ""
  property int shakeOffset: 0

  property string fontFamily: Style.font.menuFamily
  property color accent: Color.polkit.accent
  property color background: Color.polkit.background
  property color foreground: Color.polkit.text
  property color textError: Color.polkit.textError
  property var borderSpec: Border.surfaceSpec("polkit", errorFlash ? "border-error" : "border", errorFlash ? Color.polkit.borderError : Color.polkit.border, Math.max(1, Style.space(2)), "border-alpha")
  readonly property int contentMargin: Style.spacing.panelPadding
  readonly property int fieldHeight: Math.max(Style.space(42), Style.spacing.controlHeight)
  readonly property bool wantsPin: mode === "getpin"
  readonly property string shownError: localError || error
  readonly property int cardWidth: Math.min(Style.space(420), Math.max(Style.space(260), panel.width - Style.gapsOut * 2))

  function open(req) {
    closeTimer.stop()
    closing = false
    mode = String(req.type || "getpin")
    title = String(req.title || "")
    desc = String(req.desc || "")
    prompt = String(req.prompt || "")
    error = String(req.error || "")
    okLabel = String(req.ok || "")
    cancelLabel = String(req.cancel || "")
    notokLabel = String(req.notok || "")
    repeatPrompt = String(req.repeat || "")
    repeatError = String(req.repeatError || "")
    // Capped at one day, like the client does: Timer.interval is an int.
    timeout = Math.min(86400, Math.max(0, Math.floor(Number(req.timeout) || 0)))
    localError = ""
    clearInputs()
    shown = true
    if (timeout > 0) {
      timeoutTimer.interval = timeout * 1000
      timeoutTimer.restart()
    }
    if (error) triggerFailureFeedback()
    Qt.callLater(refocus)
  }

  // Hides the dialog without answering (the client went away).
  function close() {
    timeoutTimer.stop()
    clearInputs()
    if (!shown) return
    closing = true
    shown = false
    closeTimer.restart()
  }

  function clearInputs() {
    pinField.input.text = ""
    repeatField.input.text = ""
  }

  function respond(result) {
    if (!shown) return
    var pin = ""
    if (result === "ok" && wantsPin) {
      if (repeatPrompt && pinField.input.text !== repeatField.input.text) {
        localError = repeatError || "Passphrases don't match"
        repeatField.input.text = ""
        triggerFailureFeedback()
        return
      }
      pin = pinField.input.text
    }
    close()
    finished(result, pin)
  }

  function refocus() {
    if (!shown) return
    if (!wantsPin) keyCatcher.forceActiveFocus()
    else if (repeatPrompt && pinField.input.text.length > 0 && repeatField.input.text.length === 0 && localError) repeatField.input.forceActiveFocus()
    else if (!pinField.input.activeFocus && !repeatField.input.activeFocus) pinField.input.forceActiveFocus()
  }

  function triggerFailureFeedback() {
    errorFlash = true
    errorTimer.restart()
    shakeAnimation.restart()
    Qt.callLater(refocus)
  }

  Timer {
    id: timeoutTimer
    repeat: false
    onTriggered: root.respond("timeout")
  }

  Timer {
    id: closeTimer
    interval: 300
    repeat: false
    onTriggered: root.closing = false
  }

  Timer {
    id: errorTimer
    interval: 1200
    repeat: false
    onTriggered: root.errorFlash = false
  }

  SequentialAnimation {
    id: shakeAnimation
    NumberAnimation { target: root; property: "shakeOffset"; to: -8; duration: 35; easing.type: Easing.OutQuad }
    NumberAnimation { target: root; property: "shakeOffset"; to: 8; duration: 50; easing.type: Easing.InOutQuad }
    NumberAnimation { target: root; property: "shakeOffset"; to: 0; duration: 55; easing.type: Easing.OutQuad }
  }

  component PinField: Item {
    id: field
    property alias input: textInput
    property string placeholder: ""
    signal accepted()

    width: parent.width
    height: root.fieldHeight

    Row {
      anchors.fill: parent
      spacing: Style.space(14)

      Text {
        text: ""
        color: root.errorFlash ? root.textError : root.accent
        font.family: root.fontFamily
        font.pixelSize: Style.font.iconLarge
        width: Style.space(26)
        height: parent.height
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
      }

      Item {
        width: parent.width - Style.space(40)
        height: parent.height

        TextInput {
          id: textInput
          anchors.fill: parent
          verticalAlignment: TextInput.AlignVCenter
          activeFocusOnPress: true
          clip: true
          selectionColor: Util.alpha(root.accent, 0.45)
          selectedTextColor: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.iconLarge
          echoMode: TextInput.Password
          passwordCharacter: "•"
          inputMethodHints: Qt.ImhSensitiveData | Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
          color: root.errorFlash ? root.textError : root.foreground
          cursorVisible: activeFocus
          enabled: root.shown
          onAccepted: field.accepted()
          Keys.onPressed: function(event) {
            if (event.key === Qt.Key_Escape) {
              root.respond("cancel")
              event.accepted = true
            }
          }
        }

        Text {
          textFormat: Text.PlainText
          anchors.left: parent.left
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          text: field.placeholder
          color: root.foreground
          opacity: 0.36
          font.family: root.fontFamily
          font.pixelSize: Style.font.iconLarge
          elide: Text.ElideRight
          visible: textInput.text.length === 0
        }

        Rectangle {
          width: Math.max(1, Style.space(2))
          height: Style.space(24)
          anchors.left: parent.left
          anchors.verticalCenter: parent.verticalCenter
          color: root.foreground
          visible: textInput.activeFocus && textInput.text.length === 0
        }

        MouseArea {
          anchors.fill: parent
          onClicked: textInput.forceActiveFocus()
        }
      }
    }
  }

  PanelWindow {
    id: panel
    visible: root.shown || root.closing
    anchors { top: true; bottom: true; left: true; right: true }
    color: "transparent"
    WlrLayershell.namespace: "omarchy-pinentry"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: root.shown ? WlrKeyboardFocus.Exclusive : WlrKeyboardFocus.None
    exclusionMode: ExclusionMode.Ignore

    Rectangle {
      anchors.fill: parent
      color: Color.polkit.scrim
    }

    MouseArea {
      anchors.fill: parent
      onClicked: root.refocus()
    }

    BorderSurface {
      id: card
      width: root.cardWidth
      height: content.implicitHeight + card.contentTopInset + card.contentBottomInset
      radius: Style.cornerRadius
      anchors.centerIn: parent
      anchors.horizontalCenterOffset: root.shakeOffset
      color: root.background
      borderSpec: root.borderSpec
      padding: root.contentMargin

      MouseArea { anchors.fill: parent; onClicked: root.refocus() }

      Item {
        id: keyCatcher
        anchors.fill: parent
        focus: true

        Keys.priority: Keys.BeforeItem
        Keys.onPressed: function(event) {
          if (event.key === Qt.Key_Escape) {
            root.respond("cancel")
            event.accepted = true
          } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            root.respond("ok")
            event.accepted = true
          }
        }
      }

      Column {
        id: content
        anchors.fill: parent
        anchors.topMargin: card.contentTopInset
        anchors.rightMargin: card.contentRightInset
        anchors.bottomMargin: card.contentBottomInset
        anchors.leftMargin: card.contentLeftInset
        spacing: Style.space(10)

        Text {
          width: parent.width
          visible: text.length > 0
          textFormat: Text.PlainText
          text: root.desc
          wrapMode: Text.Wrap
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.bodySmall
        }

        Text {
          width: parent.width
          visible: text.length > 0
          textFormat: Text.PlainText
          text: root.shownError
          wrapMode: Text.Wrap
          color: root.textError
          font.family: root.fontFamily
          font.pixelSize: Style.font.bodySmall
        }

        PinField {
          id: pinField
          visible: root.wantsPin
          placeholder: root.prompt || "Passphrase"
          onAccepted: {
            if (root.repeatPrompt && repeatField.input.text.length === 0) repeatField.input.forceActiveFocus()
            else root.respond("ok")
          }
        }

        PinField {
          id: repeatField
          visible: root.wantsPin && root.repeatPrompt.length > 0
          placeholder: root.repeatPrompt
          onAccepted: root.respond("ok")
        }

        Row {
          visible: !root.wantsPin
          anchors.right: parent.right
          spacing: Style.spacing.controlGap

          Button {
            visible: root.mode === "confirm" && root.notokLabel.length > 0
            text: root.notokLabel
            bordered: true
            onClicked: root.respond("notok")
          }
          Button {
            visible: root.mode === "confirm"
            text: root.cancelLabel || "Cancel"
            bordered: true
            onClicked: root.respond("cancel")
          }
          Button {
            text: root.okLabel || "OK"
            bordered: true
            selected: true
            onClicked: root.respond("ok")
          }
        }
      }
    }

    Rectangle {
      width: Math.min(titleText.implicitWidth + Style.space(24), panel.width - Style.gapsOut * 2)
      height: Style.space(28)
      anchors.horizontalCenter: card.horizontalCenter
      anchors.bottom: card.top
      anchors.bottomMargin: Style.space(10)
      radius: Style.cornerRadius
      color: root.background

      Text {
        id: titleText
        textFormat: Text.PlainText
        anchors.fill: parent
        anchors.leftMargin: Style.space(12)
        anchors.rightMargin: Style.space(12)
        text: root.title || "GnuPG"
        color: root.foreground
        font.family: root.fontFamily
        font.pixelSize: Style.font.bodySmall
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideMiddle
      }
    }
  }
}
