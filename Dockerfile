FROM luluhow/vanilo:latest
USER root
WORKDIR /app
COPY --chown=vanilo:vanilo . .
RUN chown -R vanilo:vanilo /app
USER vanilo
EXPOSE 3000
ENV HOST=0.0.0.0
CMD ["serve"]
