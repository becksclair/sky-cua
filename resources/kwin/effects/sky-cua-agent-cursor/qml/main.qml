import QtQuick

Item {
    id: root

    required property string cursorSource

    // The desktop agent cursor renders at 2x the browser/synthetic size (the full
    // 46x48 source) for on-screen legibility.
    width: 46
    height: 48

    Image {
        anchors.fill: parent
        fillMode: Image.PreserveAspectFit
        smooth: true
        source: root.cursorSource
        sourceSize.height: 48
        sourceSize.width: 46
    }
}
