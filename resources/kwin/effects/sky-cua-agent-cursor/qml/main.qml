import QtQuick

Item {
    id: root

    required property string cursorSource

    width: 23
    height: 24

    Image {
        anchors.fill: parent
        fillMode: Image.PreserveAspectFit
        smooth: true
        source: root.cursorSource
        sourceSize.height: 24
        sourceSize.width: 23
    }
}
