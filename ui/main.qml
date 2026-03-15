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

    // ─── AppLoad backend bridge ───────────────────────────────────────────────
    AppLoad {
        id: endpoint
        applicationID: "com.nonogram.fetcher"

        onMessageReceived: (type, contents) => {
            if (type === 1) {
                // Success: "SAVED:<filename>"
                statusText.color = "#1a1a1a"
                statusText.text  = "✓ Saved to your library:\n" + contents.replace("SAVED:", "")
                busyIndicator.visible = false
                fetchButton.enabled   = true
            } else if (type === 2) {
                // Error message
                statusText.color = "#cc2200"
                statusText.text  = "✗ " + contents
                busyIndicator.visible = false
                fetchButton.enabled   = true
            } else if (type === 3) {
                // Progress update
                statusText.color = "#555555"
                statusText.text  = contents
            }
        }
    }

    // ─── Background ──────────────────────────────────────────────────────────
    Rectangle {
        anchors.fill: parent
        color: "#f5f5f0"
    }

    // ─── Content ─────────────────────────────────────────────────────────────
    ColumnLayout {
        anchors {
            horizontalCenter: parent.horizontalCenter
            top: parent.top
            topMargin: 80
        }
        width: 900
        spacing: 0

        // Title
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "Nonogram Fetcher"
            font.pixelSize: 64
            font.weight: Font.Light
            color: "#1a1a1a"
        }

        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "nonograms.org"
            font.pixelSize: 28
            color: "#888888"
            topPadding: 4
            bottomPadding: 60
        }

        // ── Separator ─────────────────────────────────────────────────────
        Rectangle { Layout.fillWidth: true; height: 1; color: "#cccccc"; Layout.bottomMargin: 50 }

        // ── Type selector ─────────────────────────────────────────────────
        SectionLabel { text: "Type" }

        OptionRow {
            id: typeSelector
            model: ["Black & White", "Color"]
            selected: 0
        }

        Item { height: 40 }

        // ── Size selector ─────────────────────────────────────────────────
        SectionLabel { text: "Grid Size" }

        OptionRow {
            id: sizeSelector
            model: ["5×5", "10×10", "15×15", "20×20", "25×25"]
            selected: 1
        }

        Item { height: 40 }

        // ── Difficulty ────────────────────────────────────────────────────
        SectionLabel { text: "Max Difficulty (rating)" }

        OptionRow {
            id: diffSelector
            model: ["Any", "Easy (1–2★)", "Medium (3★)", "Hard (4–5★)"]
            selected: 0
        }

        Item { height: 70 }

        // ── Separator ─────────────────────────────────────────────────────
        Rectangle { Layout.fillWidth: true; height: 1; color: "#cccccc"; Layout.bottomMargin: 60 }

        // ── Fetch button ──────────────────────────────────────────────────
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
                    // Build request JSON
                    var sizeMap = ["5", "10", "15", "20", "25"]
                    var payload = JSON.stringify({
                        type_bw:    typeSelector.selected === 0,
                        size:       sizeMap[sizeSelector.selected],
                        difficulty: diffSelector.selected
                    })

                    fetchButton.enabled   = false
                    busyIndicator.visible = true
                    statusText.color      = "#555555"
                    statusText.text       = "Connecting to nonograms.org…"

                    endpoint.sendMesssage(0, payload)
                }
            }
        }

        Item { height: 40 }

        // ── Busy indicator ────────────────────────────────────────────────
        BusyIndicator {
            id: busyIndicator
            Layout.alignment: Qt.AlignHCenter
            width: 64
            height: 64
            visible: false
            running: visible
        }

        // ── Status text ───────────────────────────────────────────────────
        Text {
            id: statusText
            Layout.alignment: Qt.AlignHCenter
            Layout.fillWidth: true
            text: ""
            font.pixelSize: 30
            color: "#555555"
            wrapMode: Text.WordWrap
            horizontalAlignment: Text.AlignHCenter
            topPadding: 10
        }

        Item { height: 80 }

        // ── Close button ──────────────────────────────────────────────────
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: "Close"
            font.pixelSize: 28
            color: "#999999"
            bottomPadding: 40

            MouseArea {
                anchors.fill: parent
                onClicked: root.close()
            }
        }
    }

    // ─── Reusable components ──────────────────────────────────────────────────
    component SectionLabel: Text {
        Layout.alignment: Qt.AlignLeft
        font.pixelSize: 28
        color: "#555555"
        font.weight: Font.Medium
        bottomPadding: 16
    }

    component OptionRow: RowLayout {
        id: optRow
        property var model: []
        property int selected: 0
        Layout.fillWidth: true
        spacing: 16
        bottomPadding: 4

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
