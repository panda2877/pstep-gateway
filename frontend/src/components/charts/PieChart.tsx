import React from 'react';
import type { ModelDistribution } from '../../types';

interface PieChartProps {
  data: ModelDistribution[];
}

export const PieChart: React.FC<PieChartProps> = ({ data }) => {
  const totalTokens = data.reduce((sum, d) => sum + d.tokens, 0);
  const radius = 14;
  const cx = 18;
  const cy = 18;

  const renderPaths = () => {
    let startAngle = 0;
    const paths: React.ReactNode[] = [];

    data.forEach((item, i) => {
      const percent = totalTokens > 0 ? (item.tokens / totalTokens) * 100 : 0;
      const angle = (percent / 100) * 360;

      if (angle < 0.5) return;

      const endAngle = startAngle + angle;
      const startRad = (startAngle * Math.PI) / 180;
      const endRad = (endAngle * Math.PI) / 180;

      const x1 = cx + radius * Math.cos(startRad);
      const y1 = cy + radius * Math.sin(startRad);
      const x2 = cx + radius * Math.cos(endRad);
      const y2 = cy + radius * Math.sin(endRad);

      const large = angle > 180 ? 1 : 0;
      const dAttr = `M ${cx} ${cy} L ${x1} ${y1} A ${radius} ${radius} 0 ${large} 1 ${x2} ${y2} Z`;

      paths.push(<path key={i} d={dAttr} fill={item.color} />);
      startAngle = endAngle;
    });

    return paths;
  };

  return (
    <div className="pie-wrap">
      <svg className="pie-svg" viewBox="0 0 36 36">
        {renderPaths()}
      </svg>
      <div className="pie-legend">
        {data.map((item, i) => {
          const percent = totalTokens > 0 ? (item.tokens / totalTokens) * 100 : 0;
          return (
            <div key={i} className="pie-legend-item">
              <div className="pie-legend-row">
                <div className="pie-legend-dot" style={{ background: item.color }}></div>
                <span className="pie-legend-name">{item.name}</span>
              </div>
              <span className="pie-legend-pct">{percent.toFixed(1)}%</span>
            </div>
          );
        })}
      </div>
    </div>
  );
};