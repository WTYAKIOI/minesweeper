import QtQuick 2.15
import QtQuick.Controls 2.15
import QtCharts 2.3

/**
 * 进步历程图 - 浅色主题
 * 仅展示"创纪录"的数据点，X轴为指标值，Y轴为时间（从下往上推进）
 * 顶部有指标选择按钮组，鼠标悬停显示详情
 */

ChartView {
    id: chartView
    title: "进步历程"
    titleColor: "#333333"
    titleFont.pixelSize: 14
    backgroundColor: "#ffffff"
    legend.visible: false
    antialiasing: true
    margins.top: 30
    margins.bottom: 20
    margins.left: 10
    margins.right: 30

    property string currentMetric: "rtime"
    property var metricsInfo: ({
            "rtime": "用时(s)",
            "3bvs": "3BV/s",
            "ioe": "IOE",
            "thrp": "thrp",
            "corr": "corr",
            "ces": "ces",
            "cls": "cls"
        })
    property var metricsKeys: ["rtime", "3bvs", "ioe", "thrp", "corr", "ces", "cls"]
    property var progressData: []
    property var _axisX: null
    property var _axisY: null
    property string _hoverText: ""

    // ── 指标选择器 ──
    Row {
        id: metricBar
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.topMargin: 8
        anchors.rightMargin: 30
        spacing: 4
        z: 10

        Repeater {
            model: chartView.metricsKeys

            Button {
                id: metricBtn
                property string metricKey: modelData
                property bool isActive: chartView.currentMetric === metricKey
                text: chartView.metricsInfo[metricKey] || metricKey
                padding: 4
                background: Rectangle {
                    color: metricBtn.isActive ? "#1565c0" : (metricBtn.pressed ? "#e3f2fd" : "#f5f5f5")
                    radius: 4
                    border.color: metricBtn.isActive ? "#1565c0" : "#cccccc"
                    implicitHeight: 24
                }
                contentItem: Text {
                    text: parent.text
                    color: metricBtn.isActive ? "#ffffff" : "#333333"
                    font.pixelSize: 11
                    font.bold: metricBtn.isActive
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                }
                onClicked: {
                    chartView.currentMetric = metricKey;
                    chartView.refresh();
                }
            }
        }
    }

    function refresh() {
        try {
            var json = JSON.parse(bridge.getProgress(currentMetric, root.currentLevel, root.currentMode, root.startUs, root.endUs));
            progressData = json;
            updateChart();
        } catch (e) {
            console.warn("ProgressChart refresh error:", e);
        }
    }

    function formatTs(tsSec) {
        var dt = new Date(tsSec * 1000);
        var y = dt.getFullYear();
        var mo = dt.getMonth() + 1;
        var d = dt.getDate();
        var h = dt.getHours();
        var mi = dt.getMinutes();
        var s = dt.getSeconds();
        return y + "-" + (mo < 10 ? "0" : "") + mo + "-" + (d < 10 ? "0" : "") + d + " " + (h < 10 ? "0" : "") + h + ":" + (mi < 10 ? "0" : "") + mi + ":" + (s < 10 ? "0" : "") + s;
    }

    function updateChart() {
        chartView.removeAllSeries();
        if (_axisX) {
            _axisX.destroy();
            _axisX = null;
        }
        if (_axisY) {
            _axisY.destroy();
            _axisY = null;
        }

        if (!progressData || progressData.length === 0) {
            chartView.title = "进步历程（无数据）";
            return;
        }

        var data = progressData;
        var len = data.length;

        chartView.title = "进步历程 — " + (metricsInfo[currentMetric] || currentMetric);

        // 创建序列（不绑定轴，后面 setAxis）
        var series = chartView.createSeries(ChartView.SeriesTypeLine, "progress");
        series.color = "#1565c0";
        series.width = 2;
        series.pointsVisible = true;

        var minVal = Infinity, maxVal = -Infinity;
        var minTs = Infinity, maxTs = -Infinity;

        for (var i = 0; i < len; i++) {
            var val = parseFloat(data[i].metric_value) || 0;
            var ts = parseFloat(data[i].ts_sec) || 0;
            series.append(ts * 1000, val);  // X=时间（毫秒）, Y=指标值
            if (val < minVal)
                minVal = val;
            if (val > maxVal)
                maxVal = val;
            if (ts < minTs)
                minTs = ts;
            if (ts > maxTs)
                maxTs = ts;
        }

        // X轴：时间（DateTimeAxis 显示 yy-MM-dd hh:mm:ss）
        _axisX = Qt.createQmlObject('import QtCharts 2.3; DateTimeAxis {}', chartView);
        _axisX.min = new Date(minTs * 1000);
        _axisX.max = new Date(maxTs * 1000);
        _axisX.format = "yy-MM-dd hh:mm:ss";
        _axisX.labelsColor = "#666666";
        _axisX.labelsFont.pixelSize = 9;
        _axisX.gridVisible = true;
        _axisX.gridLineColor = "#f0f0f0";
        _axisX.tickCount = Math.min(len, 6);
        chartView.setAxisX(_axisX, series);

        // Y轴：指标值
        _axisY = Qt.createQmlObject('import QtCharts 2.3; ValueAxis {}', chartView);
        var yMargin = (maxVal - minVal) * 0.1 || 1;
        _axisY.min = Math.max(minVal - yMargin, 0);
        _axisY.max = maxVal + yMargin;
        _axisY.labelsColor = "#333333";
        _axisY.labelsFont.pixelSize = 10;
        _axisY.titleText = metricsInfo[currentMetric] || currentMetric;
        _axisY.gridVisible = true;
        _axisY.gridLineColor = "#e0e0e0";
        chartView.setAxisY(_axisY, series);
    }

    // ── 悬停显示详情 ──
    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        onPositionChanged: {
            if (!progressData || progressData.length === 0 || !_axisX)
                return;
            var plotX = mouseX - chartView.plotArea.x;
            var plotW = chartView.plotArea.width;
            if (plotW <= 0)
                return;
            // 根据 X 坐标映射到时间戳，找最近的数据点
            var minMs = _axisX.min.getTime();
            var maxMs = _axisX.max.getTime();
            var targetTs = (minMs + (plotX / plotW) * (maxMs - minMs)) / 1000;
            var bestIdx = -1;
            var bestDist = Infinity;
            for (var i = 0; i < progressData.length; i++) {
                var ts = parseFloat(progressData[i].ts_sec) || 0;
                var dist = Math.abs(ts - targetTs);
                if (dist < bestDist) {
                    bestDist = dist;
                    bestIdx = i;
                }
            }
            if (bestIdx >= 0) {
                var d = progressData[bestIdx];
                _hoverText = "#" + (d.replay_id || "") + "  " + formatTs(d.ts_sec) + "  " + (metricsInfo[currentMetric] || "") + ": " + parseFloat(d.metric_value).toFixed(3);
                hoverTip.mouseX = mouseX;
                hoverTip.mouseY = mouseY;
            } else {
                _hoverText = "";
            }
        }
        onExited: {
            _hoverText = "";
        }
    }

    // 悬停提示
    Rectangle {
        id: hoverTip
        visible: _hoverText !== ""
        x: Math.min(mouseX + 12, parent.width - width - 8)
        y: Math.max(mouseY - 30, 4)
        property real mouseX: 0
        property real mouseY: 0
        color: "#ffffffee"
        border.color: "#cccccc"
        radius: 4
        width: hoverLabel.width + 12
        height: hoverLabel.height + 8
        z: 20

        Text {
            id: hoverLabel
            anchors.centerIn: parent
            text: chartView._hoverText
            font.pixelSize: 11
            color: "#333333"
        }
    }
}
