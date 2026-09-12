// The JavaScript behind `foreign import` in Data.CsvStats.
//
// The compiler has not seen any of this and takes the declared types on
// trust, which is what the file pane warns about.

export const parseNumber = (text) => {
  const value = Number.parseFloat(text);
  return Number.isNaN(value) ? 0 : value;
};

export const nowMilliseconds = () => Date.now();
