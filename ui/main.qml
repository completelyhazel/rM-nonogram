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
            if (type === 1) {
                // Success: backend has finished saving the PDF.
                statusText.color = "#1a1a1a"
                statusText.text  = "Saved to library:\n"
                                 + contents.replace("SAVED:", "")
                                 + "\n\nYou can close the app."
                fetchButton.enabled = true
                // No auto-close timer: the user decides when to leave.
                // (An auto-close killed the backend mid-download in some cases.)
            } else if (type === 2) {
                statusText.color = "#cc2200"
                statusText.text  = "Error: " + contents
                fetchButton.enabled = true
            } else if (type === 3) {
                // Progress update from the worker thread
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
            topMargin: 80
        }
        width: 900
        spacing: 0

        // ── Title ─────────────────────────────────────────────────────────────
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "Nonogram Fetcher"
            font.pixelSize: 64
            font.weight: Font.Light
            color: "#1a1a1a"
        }

        Item { height: 4 }

        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "nonograms.org"
            font.pixelSize: 28
            color: "#888888"
        }

        Item { height: 60 }

        Rectangle { Layout.fillWidth: true; height: 1; color: "#cccccc" }

        Item { height: 50 }

        // ── Type ──────────────────────────────────────────────────────────────
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
            model: ["Black & White", "Color"]
            selected: 0
        }

        Item { height: 40 }

        // ── Grid size ─────────────────────────────────────────────────────────
        Text {
            Layout.alignment: Qt.AlignLeft
            text: "Grid Size"
            font.pixelSize: 28
            color: "#555555"
            font.weight: Font.Medium
        }

        Item { height: 16 }

        OptionRow {
            id: sizeSelector
            model: ["XSmall", "Small", "Medium", "Large", "XLarge"]
            selected: 1
        }

        Item { height: 40 }

        // ── Difficulty ────────────────────────────────────────────────────────
        Text {
            Layout.alignment: Qt.AlignLeft
            text: "Difficulty"
            font.pixelSize: 28
            color: "#555555"
            font.weight: Font.Medium
        }

        Item { height: 16 }

        OptionRow {
            id: diffSelector
            model: ["Any", "Easy", "Medium", "Hard"]
            selected: 0
        }

        Item { height: 70 }

        Rectangle { Layout.fillWidth: true; height: 1; color: "#cccccc" }

        Item { height: 60 }

        // ── Fetch button ──────────────────────────────────────────────────────
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
                text: "Fetch Nonogram"
                font.pixelSize: 36
                color: "#ffffff"
                font.weight: Font.Medium
            }

            MouseArea {
                id: fetchArea
                anchors.fill: parent
                enabled: fetchButton.enabled
                onClicked: {
                    var sizeMap = ["5", "10", "15", "20", "25"]
                    var payload = JSON.stringify({
                        type_bw:    typeSelector.selected === 0,
                        size:       sizeMap[sizeSelector.selected],
                        difficulty: diffSelector.selected
                    })
                    fetchButton.enabled = false
                    statusText.color    = "#555555"
                    statusText.text     = "Connecting to nonograms.org…"
                    endpoint.sendMessage(0, payload)
                }
            }
        }

        Item { height: 40 }

        // Status / progress text — no animated spinner (e-ink unfriendly)
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

        Item { height: 80 }

        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "Close"
            font.pixelSize: 28
            color: "#999999"

            MouseArea {
                anchors.fill: parent
                onClicked: root.close()
            }
        }

        Item { height: 40 }
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
