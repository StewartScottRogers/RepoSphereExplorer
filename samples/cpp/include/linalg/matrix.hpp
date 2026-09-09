// A small dense matrix template with dimensions checked at run time.
//
// Sizes are values, not template parameters, because a matrix whose shape
// is only known once a file has been read is the ordinary case. What is
// checked is that two matrices agree before they are multiplied, and the
// failure is an exception naming both shapes rather than undefined
// behaviour.

#ifndef LINALG_MATRIX_HPP
#define LINALG_MATRIX_HPP

#include <cstddef>
#include <initializer_list>
#include <stdexcept>
#include <string>
#include <vector>

namespace linalg {

/// Thrown when two matrices that must agree do not.
class DimensionMismatch : public std::runtime_error {
public:
    explicit DimensionMismatch(const std::string& what) : std::runtime_error(what) {}
};

/// A dense, row-major matrix of `T`.
template <typename T>
class Matrix {
public:
    Matrix(std::size_t rows, std::size_t cols);

    /// Rows given literally, which is how a test writes one. Throws
    /// DimensionMismatch if the rows are not all the same length.
    Matrix(std::initializer_list<std::initializer_list<T>> rows);

    T& at(std::size_t row, std::size_t col);
    const T& at(std::size_t row, std::size_t col) const;

    [[nodiscard]] std::size_t rows() const noexcept;
    [[nodiscard]] std::size_t cols() const noexcept;

    [[nodiscard]] Matrix transposed() const;

private:
    std::size_t rows_;
    std::size_t cols_;
    std::vector<T> cells_;
};

/// Throws DimensionMismatch unless left.cols() == right.rows().
template <typename T>
Matrix<T> operator*(const Matrix<T>& left, const Matrix<T>& right);

/// A square matrix with ones down the diagonal.
struct Identity {
    static Matrix<double> of(std::size_t size);
};

}  // namespace linalg

#endif  // LINALG_MATRIX_HPP
