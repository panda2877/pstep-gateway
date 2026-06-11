import React from 'react';

interface CardProps {
  children: React.ReactNode;
  className?: string;
  flat?: boolean;
}

export const Card: React.FC<CardProps> = ({
  children,
  className = '',
  flat = false,
}) => {
  const classes = ['card', flat ? 'card-flat' : '', className].filter(Boolean).join(' ');
  return <div className={classes}>{children}</div>;
};