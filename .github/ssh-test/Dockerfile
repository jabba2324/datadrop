# A throwaway SSH server for the SSH tunnel tests in CI (see .github/workflows/ci.yml).
# Build it next to an authorized_keys file. The password is for tests only.
FROM alpine:3.20
ARG PASSWORD=ci-only-ssh
RUN apk add --no-cache openssh-server \
 && ssh-keygen -A \
 && adduser -D dd && echo "dd:${PASSWORD}" | chpasswd \
 && mkdir -p /home/dd/.ssh && chown dd /home/dd/.ssh && chmod 700 /home/dd/.ssh \
 && sed -i 's/^AllowTcpForwarding no/AllowTcpForwarding yes/' /etc/ssh/sshd_config \
 && printf 'Port 2222\nAllowTcpForwarding yes\nPasswordAuthentication yes\nPermitRootLogin no\n' >> /etc/ssh/sshd_config
COPY authorized_keys /home/dd/.ssh/authorized_keys
RUN chown dd /home/dd/.ssh/authorized_keys && chmod 600 /home/dd/.ssh/authorized_keys
CMD ["/usr/sbin/sshd", "-D", "-e"]
