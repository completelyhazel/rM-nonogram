import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15
import net.asivery.AppLoad 1.0

Item {
    id: root
    width: 1404
    height: 1872

    signal close
    function unloading() {
        endpoint.terminate()
    }

    AppLoad {
        id: endpoint
        applicationID: "com.nonogram.fetcher"

        onMessageReceived: (type, contents) => {
            console.log("[qml] received type=" + type + " contents=" + contents)
            if (type == 1) { // success
                statusText.color = "#1a6b1a"
                statusText.text  = "Saved successfully!\n\nRestarting library…"
                fetchButton.enabled = false

            } else if (type == 2) { // erro
                statusText.color    = "#cc2200"
                statusText.text     = "Error: " + contents
                fetchButton.enabled = true

            } else if (type == 3) { // info
                statusText.color = "#555555"
                statusText.text  = contents
            }
        }
    }

    Rectangle {
        anchors.fill: parent
        color: "#f5f5f0"
    }

    ColumnLayout {
        anchors {
            horizontalCenter: parent.horizontalCenter
            top: parent.top
            topMargin: 120
        }
        width: 900
        spacing: 0

        // decoration
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "rM-nonogram"
            font.pixelSize: 64
            font.weight: Font.Light
            color: "#1a1a1a"
        }

        Item { height: 4 }

        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "getting puzzles from nonograms.org !! :3"
            font.pixelSize: 28
            color: "#747474"
        }

        Item { height: 60 }

        Rectangle { Layout.fillWidth: true; height: 1; color: "#cccccc" }

        Item { height: 50 }

        // type
        Text {
            Layout.alignment: Qt.AlignLeft
            text: "Type"
            font.pixelSize: 28
            color: "#555555"
            font.weight: Font.Medium
        }

        Item { height: 16 }

        OptionRow {
            id: typeSelector
            model: ["Black & White", "Colored"]
            selected: 0
        }

        Item { height: 40 }

        // grid size
        Text {
            Layout.alignment: Qt.AlignLeft
            text: "Max Size: 15"
            font.pixelSize: 28
            color: "#555555"
            font.weight: Font.Medium
            id: maxText
        }

        Item { height: 16 }

        Slider {
            id: maxSize
            Layout.fillWidth: true
            Layout.maximumWidth = 200
            from: 5
            value: 15
            to: 200
            stepSize: 5
            scale: 3
            snapMode: Slider.SnapAlways

            onMoved: {
               maxText.text = "Max Size: " + maxSize.value
            }
        }

        Item { height: 16 }

        Text {
            Layout.alignment: Qt.AlignLeft
            text: "Min Size: 5"
            font.pixelSize: 28
            color: "#555555"
            font.weight: Font.Medium
            id: minText
        }

        Item { height: 16 }

        Slider {
            id: minSize
            Layout.fillWidth: true
            Layout.maximumWidth = 200
            from: 5
            value: 5
            to: 200
            scale: 3
            stepSize: 5
            snapMode: Slider.SnapAlways
            onMoved: {
                minText.text = "Min Size: " + minSize.value
            }
        }

        Item { height: 40 }

        // force 5x5

       CheckBox {
            font.pixelSize: 28
            text: qsTr("Force size to multiples of 5")
            id: fiveMultiple
        }

        Item { height: 70 }

        Rectangle { Layout.fillWidth: true; height: 1; color: "#cccccc" }

        Item { height: 60 }

        // button
        Rectangle {
            id: fetchButton
            Layout.alignment: Qt.AlignHCenter
            width: 480
            height: 100
            radius: 8
            color: enabled ? (fetchArea.pressed ? "#111111" : "#1a1a1a") : "#aaaaaa"

            property bool enabled: true

            Text {
                anchors.centerIn: parent
                text: "Download Nonogram"
                font.pixelSize: 36
                color: "#ffffff"
                font.weight: Font.Medium
            }

            MouseArea {
                id: fetchArea
                anchors.fill: parent
                enabled: fetchButton.enabled
                onClicked: {
                    var payload = JSON.stringify({
                        type_bw: typeSelector.selected === 0,
                        min_size: minSize.value,
                        max_size: maxSize.value,
                        five_multiple: fiveMultiple.checkState === Qt.Checked
                    })
                    fetchButton.enabled = false
                    statusText.color = "#555555"
                    statusText.text = "Connecting to nonograms.org…"
                    endpoint.sendMessage(0, payload)
                }
            }
        }

        Item { height: 40 }

        Text {
            id: statusText
            Layout.alignment: Qt.AlignHCenter
            Layout.fillWidth: true
            text: ""
            font.pixelSize: 30
            color: "#555555"
            wrapMode: Text.WordWrap
            horizontalAlignment: Text.AlignHCenter
        }
    }

    component OptionRow: RowLayout {
        id: optRow
        property var model: []
        property int selected: 0
        Layout.fillWidth: true
        spacing: 16

        Repeater {
            model: optRow.model
            delegate: Rectangle {
                width: (900 - (optRow.model.length - 1) * 16) / optRow.model.length
                height: 72
                radius: 6
                color: optRow.selected === index ? "#1a1a1a" : "#e8e8e3"
                border.color: optRow.selected === index ? "#1a1a1a" : "#cccccc"
                border.width: 1

                Text {
                    anchors.centerIn: parent
                    text: optRow.model[index]
                    font.pixelSize: 26
                    color: optRow.selected === index ? "#ffffff" : "#333333"
                    font.weight: optRow.selected === index ? Font.Medium : Font.Normal
                }

                MouseArea {
                    anchors.fill: parent
                    onClicked: optRow.selected = index
                }
            }
        }
    }
}
