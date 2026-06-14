import React from 'react';
import type { ChainNode } from '../../types';

interface FallbackChainProps {
  chain: ChainNode[];
  showError?: boolean;
}

export const FallbackChain: React.FC<FallbackChainProps> = ({ chain, showError = false }) => {
  return (
    <div className="fallback-chain">
      {chain.map((node, i) => (
        <React.Fragment key={i}>
          {i > 0 && <div className="chain-arrow">→</div>}
          <div className="chain-node">
            <span className="provider">{node.upstream}</span>
            <span className="model">{node.model}</span>
          </div>
        </React.Fragment>
      ))}
      {showError && (
        <>
          <div className="chain-arrow">→</div>
          <div className="chain-node" style={{ borderStyle: 'dashed', color: 'var(--muted)' }}>
            返回错误
          </div>
        </>
      )}
    </div>
  );
};