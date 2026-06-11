import React from 'react';

interface SelectProps extends React.SelectHTMLAttributes<HTMLSelectElement> {
  label?: string;
  options: { value: string; label: string }[];
}

export const Select: React.FC<SelectProps> = ({
  label,
  options,
  className = '',
  id,
  ...props
}) => {
  const selectId = id || props.name;
  return (
    <div className="field">
      {label && <label htmlFor={selectId}>{label}</label>}
      <select id={selectId} className={`select ${className}`} {...props}>
        {options.map((opt) => (
          <option key={opt.value} value={opt.value}>
            {opt.label}
          </option>
        ))}
      </select>
    </div>
  );
};