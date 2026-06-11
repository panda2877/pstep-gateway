import React from 'react';

interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  hint?: string;
  mono?: boolean;
}

export const Input: React.FC<InputProps> = ({
  label,
  hint,
  mono = false,
  className = '',
  id,
  ...props
}) => {
  const inputId = id || props.name;
  return (
    <div className="field">
      {label && <label htmlFor={inputId}>{label}</label>}
      <input
        id={inputId}
        className={`input ${mono ? 'input-mono' : ''} ${className}`}
        {...props}
      />
      {hint && <span className="field-hint">{hint}</span>}
    </div>
  );
};