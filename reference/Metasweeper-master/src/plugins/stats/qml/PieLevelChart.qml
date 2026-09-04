import QtQuick 2.15
import QtCharts 2.3

/**
 * 难度分布饼图 - 浅色主题
 */

ChartView {
    id: chartView
    title: "难度分布"
    titleColor: "#333333"
    titleFont.pixelSize: 14
    backgroundColor: "#ffffff"
    legend.visible: true
    legend.color: "#333333"
    legend.labelColor: "#333333"
    antialiasing: true
    margins.top: 30
    margins.bottom: 10
    margins.left: 10
    margins.right: 30

    property var pieData: []

    function refresh() {
        try {
            var json = JSON.parse(bridge.getLevelDistribution(root.currentLevel, root.currentMode, root.startUs, root.endUs));
            pieData = json;
            updateChart();
        } catch (e) {
            console.warn("PieChart refresh error:", e);
        }
    }

    function updateChart() {
        chartView.removeAllSeries();
        if (pieData.length === 0)
            return;
        var pieSeries = chartView.createSeries(ChartView.SeriesTypePieSeries, "");
        if (!pieSeries)
            return;
        pieSeries.holeSize = 0.4;
        pieSeries.startAngle = 0;
        pieSeries.endAngle = 360;

        var levelColors = {
            3: "#42a5f5",
            4: "#66bb6a",
            5: "#ef5350",
            6: "#ffa726"
        };

        var total = 0;
        for (var i = 0; i < pieData.length; i++)
            total += pieData[i].count;

        for (var i = 0; i < pieData.length; i++) {
            var level = pieData[i].level;
            var count = pieData[i].count;
            var label = root.levelNames[level] || ("Level " + level);
            var pct = total > 0 ? (count / total * 100).toFixed(1) : "0";
            var slice = pieSeries.append(label + " " + pct + "%", count);
            if (!slice)
                continue;
            slice.color = levelColors[level] || "#90a4ae";
            slice.borderColor = "#ffffff";
            slice.borderWidth = 2;
            slice.labelVisible = true;
            slice.labelColor = "#333333";
            slice.labelFont.pixelSize = 11;
            slice.labelPosition = 1;
        }
    }
}
