import React from 'react';

interface BadgeProps {
  variant?: 'default' | 'success' | 'warn' | 'danger';
  children: React.ReactNode;
  className?: string;
}

export const Badge: React.FC<BadgeProps> = ({
  variant = 'default',
  children,
  className = '',
}) => {
  const classes = [
    'badge',
    variant !== 'default' ? `badge-${variant}` : '',
    className,
  ].filter(Boolean).join(' ');

  return <span className={classes}>{children}</span>;
};