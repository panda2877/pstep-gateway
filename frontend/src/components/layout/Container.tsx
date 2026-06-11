import React from 'react';

interface ContainerProps {
  children: React.ReactNode;
}

export const Container: React.FC<ContainerProps> = ({ children }) => {
  return <main className="container">{children}</main>;
};