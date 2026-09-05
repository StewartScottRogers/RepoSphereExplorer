#include <iostream>
#include <string>

namespace greeting {

class Greeter {
public:
    explicit Greeter(std::string name) : name_(std::move(name)) {}

    void greet() const {
        std::cout << "Hello, " << name_ << "!" << std::endl;
    }

private:
    std::string name_;
};

}  // namespace greeting

int main() {
    greeting::Greeter greeter("world");
    greeter.greet();
    return 0;
}
