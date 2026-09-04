import QtQuick 2.15
import QtCharts 2.3

/**
 * 时间分布柱状图 - 浅色主题
 */

ChartView {
    id: chartView
    title: "时间分布"
    titleColor: "#333333"
    titleFont.pixelSize: 14
    backgroundColor: "#ffffff"
    legend.visible: false
    antialiasing: true
    margins.top: 30
    margins.bottom: 20
    margins.left: 10
    margins.right: 30

    property var distData: []
    property var _axisX: null
    property var _axisY: null

    function refresh() {
        try {
            var json = JSON.parse(bridge.getTimeDistribution(root.currentLevel, root.currentMode, root.startUs, root.endUs));
            distData = json;
            updateChart();
        } catch (e) {
            console.warn("DistChart refresh error:", e);
        }
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

        if (distData.length === 0)
            return;
        var barSeries = chartView.createSeries(ChartView.SeriesTypeBarSeries, "");
        var categories = [];
        var maxCount = 0;
        var values = [];

        for (var i = 0; i < distData.length; i++) {
            var bucket = parseFloat(distData[i].time_bucket) || 0;
            var count = parseInt(distData[i].count) || 0;
            values.push(count);
            categories.push(Math.round(bucket) + "s");
            if (count > maxCount)
                maxCount = count;
        }

        var barSet = barSeries.append("局数", values);
        if (barSet) {
            barSet.color = "#42a5f5";
            barSet.borderColor = "#1e88e5";
        }
        barSeries.barWidth = 0.8;

        _axisX = Qt.createQmlObject('import QtCharts 2.3; BarCategoryAxis {}', chartView);
        _axisX.categories = categories;
        _axisX.labelsColor = "#666666";
        _axisX.labelsFont.pixelSize = 10;
        _axisX.gridVisible = false;
        chartView.setAxisX(_axisX, barSeries);

        _axisY = Qt.createQmlObject('import QtCharts 2.3; ValueAxis {}', chartView);
        _axisY.min = 0;
        _axisY.max = Math.max(maxCount * 1.1, 1);
        _axisY.labelsColor = "#666666";
        _axisY.gridVisible = true;
        _axisY.gridLineColor = "#e0e0e0";
        chartView.setAxisY(_axisY, barSeries);
    }
}
