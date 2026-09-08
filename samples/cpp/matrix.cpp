// A small dense matrix with the operator overloads that make it usable,
// plus the RAII and template idioms a C++ preview should show off.
#include <algorithm>
#include <initializer_list>
#include <iostream>
#include <numeric>
#include <stdexcept>
#include <vector>

namespace linalg {

class DimensionMismatch : public std::runtime_error {
public:
    explicit DimensionMismatch(const std::string &what)
        : std::runtime_error(what) {}
};

template <typename T>
class Matrix {
public:
    Matrix(std::size_t rows, std::size_t cols)
        : rows_(rows), cols_(cols), cells_(rows * cols, T{}) {}

    Matrix(std::initializer_list<std::initializer_list<T>> rows) {
        rows_ = rows.size();
        cols_ = rows.begin()->size();
        for (const auto &row : rows) {
            if (row.size() != cols_) {
                throw DimensionMismatch("ragged initializer list");
            }
            cells_.insert(cells_.end(), row.begin(), row.end());
        }
    }

    T &at(std::size_t row, std::size_t col) { return cells_[row * cols_ + col]; }
    const T &at(std::size_t row, std::size_t col) const { return cells_[row * cols_ + col]; }

    std::size_t rows() const noexcept { return rows_; }
    std::size_t cols() const noexcept { return cols_; }

    Matrix transposed() const {
        Matrix out(cols_, rows_);
        for (std::size_t r = 0; r < rows_; ++r) {
            for (std::size_t c = 0; c < cols_; ++c) {
                out.at(c, r) = at(r, c);
            }
        }
        return out;
    }

    T trace() const {
        if (rows_ != cols_) {
            throw DimensionMismatch("trace of a non-square matrix");
        }
        T sum{};
        for (std::size_t i = 0; i < rows_; ++i) {
            sum += at(i, i);
        }
        return sum;
    }

private:
    std::size_t rows_ = 0;
    std::size_t cols_ = 0;
    std::vector<T> cells_;
};

template <typename T>
Matrix<T> operator*(const Matrix<T> &left, const Matrix<T> &right) {
    if (left.cols() != right.rows()) {
        throw DimensionMismatch("inner dimensions disagree");
    }
    Matrix<T> out(left.rows(), right.cols());
    for (std::size_t r = 0; r < left.rows(); ++r) {
        for (std::size_t c = 0; c < right.cols(); ++c) {
            T sum{};
            for (std::size_t k = 0; k < left.cols(); ++k) {
                sum += left.at(r, k) * right.at(k, c);
            }
            out.at(r, c) = sum;
        }
    }
    return out;
}

template <typename T>
std::ostream &operator<<(std::ostream &out, const Matrix<T> &matrix) {
    for (std::size_t r = 0; r < matrix.rows(); ++r) {
        for (std::size_t c = 0; c < matrix.cols(); ++c) {
            out << matrix.at(r, c) << (c + 1 == matrix.cols() ? '\n' : ' ');
        }
    }
    return out;
}

struct Identity {
    static Matrix<double> of(std::size_t size) {
        Matrix<double> out(size, size);
        for (std::size_t i = 0; i < size; ++i) {
            out.at(i, i) = 1.0;
        }
        return out;
    }
};

}  // namespace linalg

int main() {
    using linalg::Matrix;

    Matrix<double> a{{1.0, 2.0}, {3.0, 4.0}};
    const Matrix<double> product = a * linalg::Identity::of(2);

    std::cout << product;
    std::cout << "trace: " << product.trace() << '\n';

    try {
        Matrix<double> ragged{{1.0, 2.0}, {3.0}};
    } catch (const linalg::DimensionMismatch &err) {
        std::cerr << "rejected: " << err.what() << '\n';
    }

    return 0;
}
